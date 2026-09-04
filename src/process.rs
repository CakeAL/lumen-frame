use libvips::{Result, VipsImage, ops};

use crate::params::{Position, WatermarkParams};

// 计算画布大小
pub fn cal_canvas_size(img_w: i32, img_h: i32, params: &WatermarkParams) -> (i32, i32) {
    let mut canvas_h = (img_h as f64 * (1.0 + params.border_ratio.0)).round() as i32;
    let mut canvas_w = if params.border_equal {
        // 边框等宽
        img_w + (canvas_h - img_h)
    } else {
        (img_w as f64 * (1.0 + params.border_ratio.1)).round() as i32
    };
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

// 计算图片放置位置
/// 计算图片放置位置
pub fn cal_image_coordinates(
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

/// 生成画布
pub fn new_canvas(
    canvas_w: i32,
    canvas_h: i32,
    img: &VipsImage,
    params: &WatermarkParams,
) -> Result<VipsImage> {
    let (img_w, img_h) = (img.get_width(), img.get_height());
    if params.solid_background {
        // 纯色背景
        let [r, g, b] = params.background;
        let background = ops::black(canvas_w, canvas_h)?;
        ops::linear(&background, &mut [1.0], &mut [r as f64, g as f64, b as f64])
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
    // 阴影需要比照片稍微大一点，否则 blur 会被边界截掉。
    let shadow_margin = (shadow_sigma * 2.0).ceil() as i32;
    let shadow_w = img_w + shadow_margin * 2;
    let shadow_h = img_h + shadow_margin * 2;
    // 创建阴影mask
    let shadow_mask = if params.border_radius > 0.0 {
        let radius = (img_h as f64 * params.border_radius).round() as i32;
        let shadow_radius = radius + shadow_margin;

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
                    fill="black"/>
            </svg>
            "#,
            w = shadow_w,
            h = shadow_h,
            r = shadow_radius,
        );

        ops::svgload_buffer(svg.as_bytes())?
    } else {
        // 没有圆角时直接用矩形
        let shadow = ops::black(shadow_w, shadow_h)?;
        ops::linear(&shadow, &mut [0.0], &mut [255.0])?
    };

    // 确保 mask 为单通道
    let shadow_mask = if shadow_mask.get_bands() > 1 {
        ops::extract_band(&shadow_mask, 0)?
    } else {
        shadow_mask
    };
    // 对 mask 进行高斯模糊
    let shadow_mask = ops::gaussblur(&shadow_mask, shadow_sigma)?;
    // 生成黑色 RGB
    let shadow_black = ops::black(shadow_w, shadow_h)?;
    // black() 本身就是单通道，因此直接复制成 3 个 band
    let shadow_rgb =
        ops::bandjoin(&mut [shadow_black.clone(), shadow_black.clone(), shadow_black])?;
    // 根据 opacity 调整 Alpha
    let shadow_alpha = ops::linear(&shadow_mask, &mut [params.shadow_opacity], &mut [0.0])?;
    // RGB + Alpha → RGBA
    let shadow = ops::bandjoin(&mut [shadow_rgb, shadow_alpha])?;
    let shadow_x = img_x - shadow_margin;
    let shadow_y = img_y - shadow_margin;

    println!(
        "canvas: {}x{}, {} bands, {:?}",
        canvas.get_width(),
        canvas.get_height(),
        canvas.get_bands(),
        canvas.get_format(),
    );

    println!(
        "shadow: {}x{}, {} bands, {:?}",
        shadow.get_width(),
        shadow.get_height(),
        shadow.get_bands(),
        shadow.get_format(),
    );

    println!("shadow position: {}, {}", shadow_x, shadow_y);

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
