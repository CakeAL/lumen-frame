//! 固定种子的灰度颗粒，只处理背景 RGB，保留透明度。

use vips::{Result, VipsKernel};

use crate::media::{
    ensure_vips,
    vips::{VipsImage, image_op},
};

/// 输入为 sRGB RGBA；颗粒随照片尺寸等比缩放，不随预览重绘重新随机。
pub(super) fn apply(background: &VipsImage, photo_height: i32, strength: f64) -> Result<VipsImage> {
    ensure_vips();
    let (width, height) = (background.width() as i32, background.height() as i32);
    // 每 1200 像素照片高度对应一层细颗粒，导出大图时保持相近的视觉密度。
    let scale = photo_height.max(1) as f64 / 1200.0;
    let noise_width = (width as f64 / scale).ceil().max(1.0) as i32;
    let noise_height = (height as f64 / scale).ceil().max(1.0) as i32;
    let noise = image_op(|out| unsafe {
        vips_sys::vips_gaussnoise(
            out,
            noise_width,
            noise_height,
            c"mean".as_ptr(),
            0.0_f64,
            c"sigma".as_ptr(),
            strength.clamp(0.0, 1.0) * 40.0,
            c"seed".as_ptr(),
            42_i32,
            std::ptr::null::<i8>(),
        )
    })?;
    let noise = noise.resize(
        width as f64 / noise_width as f64,
        Some(height as f64 / noise_height as f64),
        Some(VipsKernel::VIPS_KERNEL_LINEAR),
    )?;
    let rgb = image_op(|out| unsafe {
        vips_sys::vips_extract_band(
            background.as_ptr(),
            out,
            0_i32,
            c"n".as_ptr(),
            3_i32,
            std::ptr::null::<i8>(),
        )
    })?;
    // libvips 将单通道噪声广播到 RGB，避免产生彩色斑点。
    let rgb = image_op(|out| unsafe {
        vips_sys::vips_add(rgb.as_ptr(), noise.as_ptr(), out, std::ptr::null::<i8>())
    })?;
    let alpha = image_op(|out| unsafe {
        vips_sys::vips_extract_band(background.as_ptr(), out, 3_i32, std::ptr::null::<i8>())
    })?;
    image_op(|out| unsafe {
        vips_sys::vips_bandjoin2(rgb.as_ptr(), alpha.as_ptr(), out, std::ptr::null::<i8>())
    })
}
