//! JPEG 内嵌缩略图。导出时必须使用最终成片，不能沿用输入照片的 EXIF 缩略图。

use anyhow::{Context as _, Result};
use vips::VipsSize;

use super::vips::{VipsImage, ensure_vips};

pub(crate) fn update_thumbnail(image: &mut VipsImage) -> Result<()> {
    ensure_vips();
    let thumbnail = image
        .thumbnail(256, 256, VipsSize::VIPS_SIZE_DOWN)
        .context("生成导出缩略图失败")?;
    // 只保留色彩配置，防止复制旧缩略图或再次编码为 Ultra HDR。
    let encoded = thumbnail
        .write_to_buffer_owned(".jpg[Q=75,keep=icc]")
        .context("编码导出缩略图失败")?;
    let bytes = encoded.as_slice();
    // SAFETY: 向有效图像写入元数据；libvips 在调用期间复制字节。
    unsafe {
        vips_sys::vips_image_set_blob_copy(
            image.as_ptr(),
            c"jpeg-thumbnail-data".as_ptr(),
            bytes.as_ptr().cast(),
            bytes.len(),
        );
    }
    Ok(())
}
