use anyhow::{Context, Result};
use libvips::{VipsImage, ops};

/// 给图片添加圆角
pub async fn add_round_corner(img: &VipsImage, border_radius: f64) -> Result<()> {
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
    let mask =
        ops::svgload_buffer(svg.as_bytes()).context("failed to create rounded corner mask")?;

    // SVG 可能产生 RGB / RGBA，确保最终只有一个 alpha mask。
    let mask = if mask.get_bands() > 1 {
        mask.extract_band(0)?
    } else {
        mask
    };

    // 原图转成 RGBA，然后把 mask 作为 alpha。
    let image = if img.get_bands() == 4 {
        let rgb = img.extract_band_with_opts(
            0,
            &ops::ExtractBandOptions {
                n: 3,
                ..Default::default()
            },
        )?;

        rgb.bandjoin(&mask)?
    } else {
        img.bandjoin(&mask)?
    };
    Ok(())
}

/// 生成阴影
pub async fn generate_shadow(
    img: &VipsImage,
    shadow_size: f64,
    shadow_opacity: f64,
    border_radius: f64,
) -> Result<VipsImage> {
    let (img_w, img_h) = (img.get_width(), img.get_height());
    let shadow_size = (img_h as f64 * shadow_size).round() as i32;
    let shadow_sigma = shadow_size as f64 / 2.0;
    // 阴影需要比照片稍微大一点，否则 blur 会被边界截掉。
    let shadow_margin = (shadow_sigma * 2.0).ceil() as i32;
    let shadow_w = img_w + shadow_margin * 2;
    let shadow_h = img_h + shadow_margin * 2;
    // 创建阴影mask
    let shadow_mask = if border_radius > 0.0 {
        let radius = (img_h as f64 * border_radius).round() as i32;
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

    let shadow_mask = if shadow_mask.get_bands() > 1 {
        shadow_mask.extract_band(0)?
    } else {
        shadow_mask
    };
    // 对mask进行模糊
     let shadow_mask =
        ops::gaussblur(&shadow_mask, shadow_sigma)?;
        // 生成黑色阴影 + alpha
    let shadow_rgb =
        ops::black(shadow_w, shadow_h)?;
    let shadow_rgb =
        ops::bandjoin(&[
            shadow_rgb.extract_band(0)?,
            shadow_rgb.extract_band(0)?,
            shadow_rgb.extract_band(0)?,
        ])?;
     // 根据 shadow_opacity 调整 alpha
    let shadow_alpha =
        ops::linear(
            &shadow_mask,
            &mut [shadow_opacity],
            &mut [0.0],
        )?;
    let shadow =
        shadow_rgb.bandjoin(&shadow_alpha)?;
    Ok(shadow)
}
