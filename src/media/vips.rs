//! libvips 的进程生命周期与基础图片解码。

use std::{ffi::CStr, path::Path, sync::OnceLock};

use anyhow::{Context as _, Result, anyhow, bail};
use libvips::{VipsApp, VipsImage, ops};

fn vips() -> &'static VipsApp {
    static VIPS: OnceLock<VipsApp> = OnceLock::new();
    VIPS.get_or_init(|| VipsApp::default("lumen-frame").expect("failed to init libvips"))
}

/// 保证 libvips 已初始化，并让进程级实例存活到程序退出。
pub fn ensure_vips() -> &'static VipsApp {
    vips()
}

/// 加载图片并按 EXIF orientation 摆正，同时保留 Ultra HDR gain map 元数据。
pub fn load_base_image(path: &Path) -> Result<VipsImage> {
    ensure_vips();
    let image =
        VipsImage::new_from_file(&path.to_string_lossy()).context("failed to load image")?;
    ops::autorot(&image).context("failed to autorotate image")
}

/// 将 vips 图像求值为连续像素内存，并在 libvips 失败时返回错误。
///
/// `libvips` crate 的同名方法没有检查 C API 返回的空指针，会在处理失败时直接 abort。
/// 这里保留相同的内存读取路径，但先验证指针与长度。
pub fn image_write_to_memory(image: &VipsImage) -> Result<Vec<u8>> {
    ensure_vips();
    let mut size = 0_u64;
    // VipsImage 目前是单字段指针包装；项目读取 gain map 时也使用同一适配方式。
    let raw = unsafe { *(image as *const VipsImage as *const *mut libvips::bindings::VipsImage) };
    let buffer = unsafe { libvips::bindings::vips_image_write_to_memory(raw, &mut size) };
    if buffer.is_null() {
        let message = unsafe {
            let error = libvips::bindings::vips_error_buffer();
            if error.is_null() {
                "未知 libvips 错误".to_owned()
            } else {
                CStr::from_ptr(error).to_string_lossy().trim().to_owned()
            }
        };
        unsafe { libvips::bindings::vips_error_clear() };
        return Err(anyhow!(message));
    }
    if size == 0 || size > isize::MAX as u64 {
        unsafe { libvips::bindings::g_free(buffer) };
        bail!("libvips 返回了非法像素缓冲区长度：{size}");
    }

    let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), size as usize) }.to_vec();
    unsafe { libvips::bindings::g_free(buffer) };
    Ok(bytes)
}
