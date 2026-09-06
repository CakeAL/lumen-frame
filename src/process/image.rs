use libvips::{Result, VipsImage, ops};

use crate::{params::{WatermarkParams}, Position};

/// 计算图片放置位置
pub fn cal_coordinates(
    canvas_w: i32,
    canvas_h: i32,
    img_w: i32,
    img_h: i32,
    params: &WatermarkParams,
) -> (i32, i32) {
    let margin_y = (img_h as f64 * params.border_ratio.0 / 2.0).round() as i32;
    let margin_x = if params.border_equal {
        margin_y
    } else {
        (img_w as f64 * params.border_ratio.1 / 2.0).round() as i32
    };
    let center_x = (canvas_w - img_w) / 2;
    let center_y = (canvas_h - img_h) / 2;
    match params.position {
        Position::Center => (center_x, center_y),
        Position::Up => (center_x, margin_y),
        Position::Right => (canvas_w - img_w - margin_x, center_y),
        Position::Bottom => (center_x, canvas_h - img_h - margin_y),
        Position::Left => (margin_x, center_y),
    }
}

/// 为图片添加圆角
pub fn add_round_corner(img: VipsImage, border_radius: f64) -> Result<VipsImage> {
    let (img_w, img_h) = (img.get_width(), img.get_height());
    let radius = (img_h as f64 * border_radius).round() as i32;
    // 使用svg生成圆角遮罩
    let svg = format!(
        r#"
            <svg xmlns="http://www.w3.org/2000/svg"
                 width="{w}"
                 height="{h}"
                 viewBox="0 0 {w} {h}">
                <rect
                    x="0"
                    y="0"
                    width="{w}"
                    height="{h}"
                    rx="{r}"
                    ry="{r}"
                    fill="white"/>
            </svg>
            "#,
        w = img_w,
        h = img_h,
        r = radius,
    );
    let mask = ops::svgload_buffer(svg.as_bytes())?;

    // SVG 可能产生 RGB / RGBA，确保最终只有一个 alpha mask。
    let mask = if mask.get_bands() > 1 {
        ops::extract_band(&mask, 0)?
    } else {
        mask
    };

    // 原图转成 RGBA，然后把 mask 作为 alpha。
    let img = if img.get_bands() == 4 {
        let img_rgb = ops::extract_band_with_opts(
            &img,
            0,
            &ops::ExtractBandOptions {
                n: 3,
                ..Default::default()
            },
        )?;
        ops::bandjoin(&mut [img_rgb, mask])?
    } else {
        ops::bandjoin(&mut [img, mask])?
    };
    Ok(img)
}
