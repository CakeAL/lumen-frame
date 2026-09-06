use libvips::{
    Result, VipsImage,
    ops::{self, BlackOptions},
};

use crate::{Position, params::WatermarkParams};

// 计算画布大小
pub fn cal_size(img_w: i32, img_h: i32, text_height: i32, text_position: Position , params: &WatermarkParams) -> (i32, i32) {
    let mut canvas_h = (img_h as f64 * (1.0 + params.border_ratio.0)).round() as i32;
    let mut canvas_w = if params.border_equal {
        // 边框等宽
        img_w + (canvas_h - img_h)
    } else {
        (img_w as f64 * (1.0 + params.border_ratio.1)).round() as i32
    };
    match text_position {
        Position::Up | Position::Bottom => {
            canvas_h += text_height;
        } 
        _ => {
            canvas_w += text_height;
        }
    }
    if let Some(aspect_ratio) = params.aspect_ratio {
        let new_h = (canvas_w as f64 / aspect_ratio.0 * aspect_ratio.1).round() as i32;
        if new_h < canvas_w {
            canvas_w = (canvas_h as f64 / aspect_ratio.1 * aspect_ratio.0).round() as i32;
        } else {
            canvas_h = new_h;
        }
    }
    (canvas_w, canvas_h)
}

/// 生成画布
pub fn new_canvas(
    canvas_w: i32,
    canvas_h: i32,
    img: &VipsImage,
    params: &WatermarkParams,
) -> Result<VipsImage> {
    let (img_w, img_h) = (img.get_width(), img.get_height());
    let canvas = if params.solid_background {
        // 纯色背景
        let [r, g, b] = params.background;
        let background = ops::black_with_opts(canvas_w, canvas_h, &BlackOptions { bands: 3 })?;
        ops::linear(
            &background,
            &mut [1.0, 1.0, 1.0],
            &mut [r as f64, g as f64, b as f64],
        )
    } else {
        // 模糊背景
        // 1. 计算缩放比例，使原图完全覆盖画布 (Cover 模式)
        let scale = f64::max(
            canvas_w as f64 / img_w as f64,
            canvas_h as f64 / img_h as f64,
        );
        // 2. 等比缩放原图
        let scaled_img = ops::resize(&img, scale)?;
        // 3. 从缩放后的图片中心裁剪出画布大小（居中裁剪）
        let (scaled_w, scaled_h) = (scaled_img.get_width(), scaled_img.get_height());
        let crop_x = (scaled_w - canvas_w) / 2;
        let crop_y = (scaled_h - canvas_h) / 2;
        let background_img = ops::extract_area(&scaled_img, crop_x, crop_y, canvas_w, canvas_h)?;
        // 4. 对裁剪后的背景图片应用高斯模糊
        ops::gaussblur(&background_img, params.blur_sigma)
    }?;
    let canvas = ops::addalpha(&canvas)?;
    let canvas = ops::cast(&canvas, ops::BandFormat::Uchar)?;
    let canvas = ops::copy_with_opts(
        &canvas,
        &ops::CopyOptions {
            interpretation: ops::Interpretation::Srgb,
            ..Default::default()
        },
    )?;
    Ok(canvas)
}

/// 为画布添加图片的阴影
pub fn add_shadow(
    canvas: VipsImage,
    img: &VipsImage,
    params: &WatermarkParams,
    img_x: i32,
    img_y: i32,
) -> Result<VipsImage> {
    let (img_w, img_h) = (img.get_width(), img.get_height());
    let shadow_size = (img_h as f64 * params.shadow_size).round() as i32;
    let shadow_sigma = shadow_size as f64 / 2.0;
    // Gaussian blur 需要足够的外围空间
    let shadow_margin = (shadow_sigma * 3.0).ceil() as i32;
    let shadow_w = img_w + shadow_margin * 2;
    let shadow_h = img_h + shadow_margin * 2;
    // 创建阴影mask
    let shadow_mask = {
        let radius = (img_h as f64 * params.border_radius).round() as i32;
        let svg = format!(
            r#"
            <svg xmlns="http://www.w3.org/2000/svg"
                 width="{shadow_w}"
                 height="{shadow_h}"
                 viewBox="0 0 {shadow_w} {shadow_h}">
                <rect
                    x="{margin}"
                    y="{margin}"
                    width="{img_w}"
                    height="{img_h}"
                    rx="{radius}"
                    ry="{radius}"
                    fill="white"/>
            </svg>
            "#,
            shadow_w = shadow_w,
            shadow_h = shadow_h,
            margin = shadow_margin,
            img_w = img_w,
            img_h = img_h,
            radius = radius,
        );

        ops::svgload_buffer(svg.as_bytes())?
    };

    // 确保 mask 为单通道
    let shadow_mask = if shadow_mask.get_bands() > 1 {
        ops::extract_band(&shadow_mask, 0)?
    } else {
        shadow_mask
    };
    // 对 mask 进行高斯模糊
    let shadow_mask = ops::gaussblur(&shadow_mask, shadow_sigma)?;
    // 生成与 mask 同尺寸的黑色 RGB（3 band）。注意不能对 VipsImage 使用 `.clone()`
    // 来复制 band：该 crate 的 Clone 是浅拷贝（不增加 GObject 引用计数），而 Drop
    // 会 unref，多次 clone 会导致 double-free / use-after-free。
    let shadow_rgb = VipsImage::new_from_image(&shadow_mask, &[0.0, 0.0, 0.0])?;
    // 根据 opacity 调整 Alpha（保持 uchar，避免 linear 默认输出 float 导致 composite2 崩溃）
    let shadow_alpha = ops::linear_with_opts(
        &shadow_mask,
        &mut [params.shadow_density],
        &mut [0.0],
        &ops::LinearOptions { uchar: true },
    )?;
    // RGB + Alpha → RGBA
    let shadow = ops::bandjoin(&mut [shadow_rgb, shadow_alpha])?;
    let shadow_x = img_x - shadow_margin;
    let shadow_y = img_y - shadow_margin;

    // println!(
    //     "canvas: {}x{} bands={} format={:?} interpretation={:?}",
    //     canvas.get_width(),
    //     canvas.get_height(),
    //     canvas.get_bands(),
    //     canvas.get_format(),
    //     canvas.get_interpretation(),
    // );
    // println!(
    //     "shadow: {}x{} bands={} format={:?} interpretation={:?}",
    //     shadow.get_width(),
    //     shadow.get_height(),
    //     shadow.get_bands(),
    //     shadow.get_format(),
    //     shadow.get_interpretation(),
    // );

    ops::composite2_with_opts(
        &canvas,
        &shadow,
        ops::BlendMode::Over,
        &ops::Composite2Options {
            x: shadow_x,
            y: shadow_y,
            ..Default::default()
        },
    )
}
