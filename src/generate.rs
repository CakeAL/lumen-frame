use anyhow::{Context, Result};
use libvips::ops::{self, Interpretation};

use crate::params::*;

/// 渲染底部水印文字为带 alpha 的 RGBA 图像。
///
/// 注意：`libvips` crate 的 `text_with_opts` 会把只读的 `autofit-dpi` 输出属性
/// 当成输入传入，与 libvips 8.18 不兼容，会导致段错误。因此这里改用
/// `ops::text`（渲染成 1-band 白字黑底掩码）→ `resize` 放大 → 上色并合成 alpha。
fn render_caption(
    caption: &str,
    params: &WatermarkParams,
    img_h: i32,
    canvas_h: i32,
) -> Result<libvips::VipsImage> {
    // 用默认字体渲染成 1-band 掩码（白色文字、黑色背景）。
    let small = ops::text(caption).context("failed to render text")?;

    // 放大到目标高度：约为底部边框高度的 55%，并限制在合理范围内。
    let border = ((canvas_h - img_h) / 2).max(1);
    let target_h = ((border as f64) * 0.55).round().clamp(12.0, 400.0) as i32;
    let scale = target_h as f64 / small.get_height() as f64;
    let mask = if (scale - 1.0).abs() > 0.01 {
        ops::resize(&small, scale).context("failed to resize text")?
    } else {
        small
    };

    // 上色并合成 alpha：RGB 用目标颜色，alpha 用文字掩码。
    let [tr, tg, tb] = params.text_params.color;
    let color = libvips::VipsImage::new_from_image(&mask, &[tr as f64, tg as f64, tb as f64])
        .context("failed to build text color")?;
    let mut bands = [color, mask];
    let rgba = ops::bandjoin(&mut bands).context("failed to build rgba text")?;

    // bandjoin 产出的 interpretation 为 multiband，标记为 sRGB 以便 composite2 合成。
    let copy_opts = ops::CopyOptions {
        interpretation: Interpretation::Srgb,
        ..Default::default()
    };
    ops::copy_with_opts(&rgba, &copy_opts).context("failed to set interpretation")
}
