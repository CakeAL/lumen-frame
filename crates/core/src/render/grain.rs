//! 固定种子的灰度颗粒，只处理背景 RGB，保留透明度。

use crate::media::vips::{extract_bands, seeded_noise};
use vips::{Result, VipsKernel};

use crate::media::{ensure_vips, vips::VipsImage};

/// 输入为 sRGB RGBA；颗粒随照片尺寸等比缩放，不随预览重绘重新随机。
pub(super) fn apply(background: &VipsImage, photo_height: i32, strength: f64) -> Result<VipsImage> {
    ensure_vips();
    let (width, height) = (background.width() as i32, background.height() as i32);
    // 每 1200 像素照片高度对应一层细颗粒，导出大图时保持相近的视觉密度。
    let scale = photo_height.max(1) as f64 / 1200.0;
    let noise_width = (width as f64 / scale).ceil().max(1.0) as i32;
    let noise_height = (height as f64 / scale).ceil().max(1.0) as i32;
    let noise = seeded_noise(noise_width, noise_height, strength.clamp(0.0, 1.0) * 40.0)?;
    let noise = noise.resize(
        width as f64 / noise_width as f64,
        Some(height as f64 / noise_height as f64),
        Some(VipsKernel::VIPS_KERNEL_LINEAR),
    )?;
    let rgb = extract_bands(&background, 0_i32, 3_i32)?;
    // libvips 将单通道噪声广播到 RGB，避免产生彩色斑点。
    let rgb = rgb.add(&noise)?;
    let alpha = background.extract_band(3_i32)?;
    rgb.bandjoin2(&alpha)
}
