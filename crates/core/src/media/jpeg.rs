//! JPEG 内嵌缩略图。导出时必须使用最终成片，不能沿用输入照片的 EXIF 缩略图。

use anyhow::{Context as _, Result};
use vips::VipsSize;
use vips_sys::VipsForeignKeep;

use super::vips::{VipsImage, ensure_vips, image_op};

pub(crate) fn update_thumbnail(image: &mut VipsImage) -> Result<()> {
    ensure_vips();
    let thumbnail = image_op(|out| unsafe {
        vips_sys::vips_thumbnail_image(
            image.as_ptr(),
            out,
            256_i32,
            c"height".as_ptr(),
            256_i32,
            c"size".as_ptr(),
            VipsSize::VIPS_SIZE_DOWN,
            std::ptr::null::<i8>(),
        )
    })
    .context("生成导出缩略图失败")?;

    let mut buffer = std::ptr::null_mut();
    let mut length = 0;
    let result = vips::code_to_result(unsafe {
        vips_sys::vips_jpegsave_buffer(
            thumbnail.as_ptr(),
            &mut buffer,
            &mut length,
            c"Q".as_ptr(),
            75_i32,
            // 缩略图只保留色彩配置，防止复制旧缩略图或再次编码为 Ultra HDR。
            c"keep".as_ptr(),
            VipsForeignKeep::VIPS_FOREIGN_KEEP_ICC,
            std::ptr::null::<i8>(),
        )
    });
    if result.is_ok() {
        unsafe {
            vips_sys::vips_image_set_blob_copy(
                image.as_ptr(),
                c"jpeg-thumbnail-data".as_ptr(),
                buffer,
                length,
            );
        }
    }
    unsafe { vips_sys::g_free(buffer) };
    result.context("编码导出缩略图失败")
}
