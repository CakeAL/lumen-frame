//! libvips 的进程生命周期、图像所有权和基础解码。

use std::{ffi::CStr, path::Path, ptr, sync::OnceLock};

use anyhow::{Context as _, Result, anyhow};
use vips::VipsInstance;

pub type VipsImage = vips::VipsImage<'static>;

fn instance() -> &'static VipsInstance {
    static VIPS: OnceLock<VipsInstance> = OnceLock::new();
    VIPS.get_or_init(|| VipsInstance::new("lumen-frame", false).expect("failed to init libvips"))
}

/// 保证 libvips 已初始化，并让进程级实例存活到程序退出。
pub fn ensure_vips() -> &'static VipsInstance {
    instance()
}

/// 接管 libvips 返回的一个非空、独占引用。
///
/// `vips` 0.1.0 尚未公开从原始指针构造图像的方法。这里固定该版本，并集中处理
/// 底层操作的返回值；升级 `vips` 时需要重新核对其 `VipsImage` 布局。
pub(crate) unsafe fn from_owned_ptr(raw: *mut vips_sys::VipsImage) -> vips::Result<VipsImage> {
    if raw.is_null() {
        return Err(vips::Error::Vips(
            vips::take_vips_error().unwrap_or_else(|| "未知 libvips 错误".into()),
        ));
    }
    assert_eq!(
        std::mem::size_of::<VipsImage>(),
        std::mem::size_of::<*mut vips_sys::VipsImage>(),
    );
    // SAFETY: vips 0.1.0 的 VipsImage 仅包含一个 NonNull 指针和零尺寸的 PhantomData。
    // 调用方必须转交一份 GObject 引用，Drop 会释放它。
    Ok(unsafe { std::mem::transmute(raw) })
}

/// 执行返回一张新图像的 vips-sys 操作，并验证状态与所有权。
pub(crate) fn image_op(
    f: impl FnOnce(*mut *mut vips_sys::VipsImage) -> i32,
) -> vips::Result<VipsImage> {
    let mut out = ptr::null_mut();
    let status = f(&mut out);
    if status != 0 {
        if !out.is_null() {
            unsafe { vips_sys::g_object_unref(out.cast()) };
        }
        return Err(vips::Error::Vips(
            vips::take_vips_error().unwrap_or_else(|| "未知 libvips 错误".into()),
        ));
    }
    unsafe { from_owned_ptr(out) }
}

/// 加载图片并按 EXIF orientation 摆正，同时保留 Ultra HDR gain map 元数据。
pub fn load_base_image(path: &Path) -> Result<VipsImage> {
    ensure_vips();
    let image = VipsImage::from_file(path).context("failed to load image")?;
    image_op(|out| unsafe { vips_sys::vips_autorot(image.as_ptr(), out, ptr::null::<i8>()) })
        .context("failed to autorotate image")
}

/// 将 vips 图像求值为连续像素内存，并在 libvips 失败时返回错误。
pub fn image_write_to_memory(image: &VipsImage) -> Result<Vec<u8>> {
    ensure_vips();
    image.write_to_memory().map_err(Into::into)
}

pub fn image_metadata_string(image: &VipsImage, name: &str) -> Result<String> {
    let name = std::ffi::CString::new(name)?;
    let mut value = ptr::null_mut();
    let status =
        unsafe { vips_sys::vips_image_get_as_string(image.as_ptr(), name.as_ptr(), &mut value) };
    vips::code_to_result(status)?;
    if value.is_null() {
        return Err(anyhow!("libvips 未返回元数据值"));
    }
    let text = unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() };
    unsafe { vips_sys::g_free(value.cast()) };
    Ok(text)
}
