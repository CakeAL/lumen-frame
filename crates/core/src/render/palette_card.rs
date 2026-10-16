//! 色块由 SVG 绘制，色号和占比复用 GPUI 字体后端；两种输出路径共用此透明图层。

use crate::{
    media::{
        ExifInfo, ensure_vips,
        vips::{VipsImage, from_owned_ptr, image_op},
    },
    watermark::{PaletteShape, Text, TextAlign, TextDirection, TextParams, WatermarkParams},
};
use anyhow::Result;

pub(crate) fn render(
    photo_size: (i32, i32),
    params: &WatermarkParams,
) -> Result<Option<VipsImage>> {
    ensure_vips();
    let card = &params.palette_card;
    let colors = card.colors();
    if !card.enabled() || colors.is_empty() {
        return Ok(None);
    }
    let scale = photo_size.1 as f64;
    let n = colors.len();
    let gap = (card.gap() * scale).round();
    let size = (card.size() * scale).round().max(1.0);
    let proportional = card.proportional();
    let mut labels = Vec::new();
    for (rgb, ratio) in &colors {
        let [r, g, b] = rgb;
        let mut lines = Vec::new();
        if card.show_hex() {
            lines.push(format!("#{r:02X}{g:02X}{b:02X}"));
        }
        if card.show_ratio() {
            lines.push(format!("{:.1}%", ratio * 100.0));
        }
        let text = Text {
            text_params: vec![
                TextParams {
                    size: card.label_size(),
                    color: Some(card.label_color()),
                    align: TextAlign::Center,
                    ..Default::default()
                };
                lines.len()
            ],
            template: lines,
        };
        labels.push(text.render_text(&ExifInfo::default(), photo_size.1, params, "")?);
    }
    let label_width = labels
        .iter()
        .flatten()
        .map(|l| l.width() as f64)
        .fold(0.0, f64::max);
    let label_height = labels
        .iter()
        .flatten()
        .map(|l| l.height() as f64)
        .fold(0.0, f64::max);
    // 占比色条的下方用等距图例，避免稀有颜色的短色段挤掉或遮挡色号。
    let block_size = if card.shape() == PaletteShape::Rounded {
        (card.width() * scale).round().max(1.0)
    } else {
        size
    };
    let cell = block_size.max(label_width);
    let length = if proportional {
        (card.length() * scale)
            .round()
            .max(n as f64 + gap * (n - 1) as f64)
            .max(label_width * n as f64 + gap * (n - 1) as f64)
    } else {
        cell * n as f64 + gap * (n - 1) as f64
    };
    let label_gap = if label_height > 0.0 {
        (card.label_size() * scale * 0.4).max(1.0)
    } else {
        0.0
    };
    let height = (size + label_gap + label_height).ceil() as i32;
    let width = length.ceil() as i32;
    let total: f64 = colors.iter().map(|(_, ratio)| ratio.max(0.0)).sum();
    let available = length - gap * (n - 1) as f64;
    let mut x = 0.0;
    let mut shapes = String::new();
    for (rgb, ratio) in &colors {
        let [r, g, b] = rgb;
        let part = if proportional {
            available
                * if total > 0.0 {
                    ratio.max(0.0) / total
                } else {
                    1.0 / n as f64
                }
        } else {
            cell
        };
        let block_width = if proportional { part } else { block_size };
        let left = x + (part - block_width) / 2.0;
        if card.shape() == PaletteShape::Circle {
            shapes += &format!(
                r#"<circle cx="{}" cy="{}" r="{}" fill="rgb({r},{g},{b})"/>"#,
                left + size / 2.0,
                size / 2.0,
                size / 2.0
            );
        } else {
            let radius = if card.shape() == PaletteShape::Rounded {
                (card.radius() * scale).min(block_width.min(size) / 2.0)
            } else {
                0.0
            };
            shapes += &format!(
                r#"<rect x="{left}" width="{block_width}" height="{size}" rx="{radius}" fill="rgb({r},{g},{b})"/>"#
            );
        }
        x += part + gap;
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}">{shapes}</svg>"#
    );
    let borrowed = vips::VipsImage::from_buffer(svg.as_bytes())?;
    let mut image = unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(borrowed.as_ptr()))? };
    let label_cell = (length - gap * (n - 1) as f64) / n as f64;
    for (ix, label) in labels.into_iter().enumerate() {
        if let Some(label) = label {
            let x = (ix as f64 * (label_cell + gap) + (label_cell - label.width() as f64) / 2.0)
                .round() as i32;
            image = image_op(|out| unsafe {
                vips_sys::vips_composite2(
                    image.as_ptr(),
                    label.as_ptr(),
                    out,
                    vips::VipsBlendMode::VIPS_BLEND_MODE_OVER,
                    c"x".as_ptr(),
                    x,
                    c"y".as_ptr(),
                    (size + label_gap).round() as i32,
                    std::ptr::null::<i8>(),
                )
            })?;
        }
    }
    if card.direction() == TextDirection::Vertical {
        image = image_op(|out| unsafe {
            vips_sys::vips_rot(
                image.as_ptr(),
                out,
                vips::VipsAngle::VIPS_ANGLE_D90,
                std::ptr::null::<i8>(),
            )
        })?;
    }
    Ok(Some(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_card_selected_colors_keep_relative_widths_and_transparent_shapes() {
        let mut params = WatermarkParams::default();
        let card = &mut params.palette_card;
        card.set_enabled(true);
        card.set_shape(PaletteShape::Square);
        card.set_proportional(true);
        card.set_length(0.4);
        card.set_gap(0.0);
        card.set_selected(vec![2, 0]);
        card.resolve(vec![
            ([255, 0, 0], 0.2),
            ([0, 255, 0], 0.4),
            ([0, 0, 255], 0.1),
        ]);
        let image = render((1000, 1000), &params).unwrap().unwrap();
        assert_eq!((image.width(), image.height()), (400, 30));
        let pixels = image::RgbaImage::from_raw(400, 30, image.write_to_memory().unwrap()).unwrap();
        assert_eq!(pixels.get_pixel(50, 15).0, [0, 0, 255, 255]);
        assert_eq!(pixels.get_pixel(200, 15).0, [255, 0, 0, 255]);
        let blue = pixels
            .rows()
            .nth(15)
            .unwrap()
            .filter(|p| p[2] > 200 && p[0] < 20)
            .count();
        assert!((blue as i32 - 133).abs() <= 1, "选色后的 1:2 占比必须保留");

        params.palette_card.set_shape(PaletteShape::Circle);
        assert!(!params.palette_card.proportional());
        let circle = render((1000, 1000), &params).unwrap().unwrap();
        assert_eq!((circle.width(), circle.height()), (60, 30));
        assert_eq!(circle.write_to_memory().unwrap()[3], 0);
        params.palette_card.set_show_hex(true);
        params.palette_card.set_show_ratio(true);
        let labeled = render((1000, 1000), &params).unwrap().unwrap();
        assert!(labeled.height() > circle.height());
        params.palette_card.set_direction(TextDirection::Vertical);
        let vertical = render((1000, 1000), &params).unwrap().unwrap();
        assert_eq!(
            (vertical.width(), vertical.height()),
            (labeled.height(), labeled.width())
        );
        params.palette_card.set_selected(Vec::new());
        assert!(render((1000, 1000), &params).unwrap().is_none());
    }
}
