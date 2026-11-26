//! libvips 的进程生命周期、基础解码及安全绑定缺少的操作。

use std::{ffi::CStr, path::Path, ptr, sync::OnceLock};

use anyhow::{Context as _, Result};
use vips::{Error, VipsBlendMode, VipsExtend, VipsInstance, VipsInterpretation, VipsPrecision};

pub type VipsImage = vips::VipsImage<'static>;

/// 保证 libvips 已初始化，并让进程级实例存活到程序退出。
pub fn ensure_vips() -> &'static VipsInstance {
    static VIPS: OnceLock<VipsInstance> = OnceLock::new();
    VIPS.get_or_init(|| VipsInstance::new("lumen-frame", false).expect("failed to init libvips"))
}

type RawImage = *mut vips_sys::VipsImage;

/// 输出指针必须为空，或持有一份可释放的引用。延迟输出只能依赖自有数据。
fn write_operation(
    target: VipsImage,
    operation: impl FnOnce(*mut RawImage) -> i32,
) -> vips::Result<VipsImage> {
    let mut raw = ptr::null_mut();
    let status = vips::code_to_result(operation(&mut raw));
    let result = status.and_then(|()| {
        if raw.is_null() {
            return Err(Error::Vips("libvips 未返回图像".into()));
        }
        // SAFETY: 两张图均存活。对于延迟输出，libvips 会自行保留输入引用；
        // 写入内存图时则立即求值，并断开与输入的像素依赖。
        vips::code_to_result(unsafe { vips_sys::vips_image_write(raw, target.as_ptr()) })
    });
    if !raw.is_null() {
        // SAFETY: 操作移交的引用只释放一次。
        unsafe { vips_sys::g_object_unref(raw.cast()) };
    }
    result.map(|()| target)
}

pub(crate) fn composite_over(
    base: &VipsImage,
    overlay: &VipsImage,
    x: i32,
    y: i32,
) -> vips::Result<VipsImage> {
    ensure_vips();
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_composite2(
            base.as_ptr(),
            overlay.as_ptr(),
            out,
            VipsBlendMode::VIPS_BLEND_MODE_OVER,
            c"x".as_ptr(),
            x,
            c"y".as_ptr(),
            y,
            ptr::null::<i8>(),
        )
    })
}

pub(crate) fn with_interpretation(
    image: &VipsImage,
    interpretation: VipsInterpretation,
) -> vips::Result<VipsImage> {
    ensure_vips();
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_copy(
            image.as_ptr(),
            out,
            c"interpretation".as_ptr(),
            interpretation,
            ptr::null::<i8>(),
        )
    })
}

pub(crate) fn extract_bands(image: &VipsImage, first: i32, count: i32) -> vips::Result<VipsImage> {
    ensure_vips();
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_extract_band(
            image.as_ptr(),
            out,
            first,
            c"n".as_ptr(),
            count,
            ptr::null::<i8>(),
        )
    })
}

pub(crate) fn linear_uchar(image: &VipsImage, a: &[f64], b: &[f64]) -> vips::Result<VipsImage> {
    ensure_vips();
    if a.is_empty() || a.len() != b.len() || a.len() > i32::MAX as usize {
        return Err(Error::Other(
            "linear_uchar requires equal nonempty a/b arrays with an i32 length".into(),
        ));
    }
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_linear(
            image.as_ptr(),
            out,
            a.as_ptr(),
            b.as_ptr(),
            a.len() as i32,
            c"uchar".as_ptr(),
            1_i32,
            ptr::null::<i8>(),
        )
    })
}

pub(crate) fn add_alpha(image: &VipsImage) -> vips::Result<VipsImage> {
    ensure_vips();
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_addalpha(image.as_ptr(), out, ptr::null::<i8>())
    })
}

pub(crate) fn constant_like(image: &VipsImage, values: &[f64]) -> vips::Result<VipsImage> {
    ensure_vips();
    if values.is_empty() || values.len() > i32::MAX as usize {
        return Err(Error::Other(
            "constant_like requires a nonempty values slice with an i32 length".into(),
        ));
    }
    write_operation(VipsImage::new()?, |out| {
        let raw = unsafe {
            vips_sys::vips_image_new_from_image(
                image.as_ptr(),
                values.as_ptr(),
                values.len() as i32,
            )
        };
        unsafe { *out = raw };
        if raw.is_null() { -1 } else { 0 }
    })
}

pub(crate) fn has_alpha(image: &VipsImage) -> bool {
    ensure_vips();
    unsafe { vips_sys::vips_image_hasalpha(image.as_ptr()) != 0 }
}

pub(crate) fn interpretation(image: &VipsImage) -> VipsInterpretation {
    ensure_vips();
    unsafe { vips_sys::vips_image_get_interpretation(image.as_ptr()) }
}

pub(crate) fn flatten_rgb(image: &VipsImage, background: [f64; 3]) -> vips::Result<VipsImage> {
    ensure_vips();
    let target = VipsImage::new()?;
    let array = unsafe { vips_sys::vips_array_double_new(background.as_ptr(), 3) };
    if array.is_null() {
        return Err(Error::Vips("无法创建 JPEG 背景色".into()));
    }
    let result = write_operation(target, |out| unsafe {
        vips_sys::vips_flatten(
            image.as_ptr(),
            out,
            c"background".as_ptr(),
            array,
            ptr::null::<i8>(),
        )
    });
    unsafe { vips_sys::vips_area_unref(array.cast()) };
    result
}

pub(crate) fn shadow_blur(mask: &VipsImage, sigma: f64) -> vips::Result<VipsImage> {
    ensure_vips();
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_gaussblur(
            mask.as_ptr(),
            out,
            sigma,
            c"min_ampl".as_ptr(),
            0.001_f64,
            c"precision".as_ptr(),
            VipsPrecision::VIPS_PRECISION_FLOAT,
            ptr::null::<i8>(),
        )
    })
}

pub(crate) fn scale_shadow(mask: &VipsImage, scale: f64) -> vips::Result<VipsImage> {
    ensure_vips();
    let interpolate = vips::VipsInterpolate::new("bilinear")?;
    // 显式对齐像素中心，避免放大时半像素偏移令四角投影不对称。
    let offset = 0.5 * (1.0 - 1.0 / scale);
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_affine(
            mask.as_ptr(),
            out,
            scale,
            0.0_f64,
            0.0_f64,
            scale,
            c"interpolate".as_ptr(),
            interpolate.as_ptr(),
            c"idx".as_ptr(),
            offset,
            c"idy".as_ptr(),
            offset,
            c"extend".as_ptr(),
            VipsExtend::VIPS_EXTEND_BLACK,
            ptr::null::<i8>(),
        )
    })
}

pub(crate) fn seeded_noise(width: i32, height: i32, sigma: f64) -> vips::Result<VipsImage> {
    ensure_vips();
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_gaussnoise(
            out,
            width,
            height,
            c"mean".as_ptr(),
            0.0_f64,
            c"sigma".as_ptr(),
            sigma,
            c"seed".as_ptr(),
            42_i32,
            ptr::null::<i8>(),
        )
    })
}

pub(crate) fn max_pair(a: &VipsImage, b: &VipsImage) -> vips::Result<VipsImage> {
    ensure_vips();
    write_operation(VipsImage::new()?, |out| unsafe {
        vips_sys::vips_maxpair(a.as_ptr(), b.as_ptr(), out, ptr::null::<i8>())
    })
}

/// 加载图片并按 EXIF orientation 摆正，同时保留 Ultra HDR gain map 元数据。
pub fn load_base_image(path: &Path) -> Result<VipsImage> {
    ensure_vips();
    VipsImage::from_file(path)
        .context("failed to load image")?
        .autorot()
        .context("failed to autorotate image")
}

/// 从素材字节解码并立即写入自有内存，断开延迟管线对输入字节的依赖。
pub fn decode_image_buffer(bytes: &[u8]) -> vips::Result<VipsImage> {
    ensure_vips();
    let borrowed = vips::VipsImage::from_buffer(bytes)?;
    let decoded = VipsImage::new_memory()?;
    vips::code_to_result(unsafe {
        vips_sys::vips_image_write(borrowed.as_ptr(), decoded.as_ptr())
    })?;
    Ok(decoded)
}

/// 若素材为 SVG，按目标高度重新栅格化，避免导出放大低分辨率中间位图；非 SVG 返回 `None`。
pub(crate) fn rerasterize_svg_at_height(
    bytes: &[u8],
    natural_height: u32,
    height: f64,
) -> vips::Result<Option<VipsImage>> {
    ensure_vips();
    let loader =
        unsafe { vips_sys::vips_foreign_find_load_buffer(bytes.as_ptr().cast(), bytes.len()) };
    let is_svg = !loader.is_null()
        && unsafe { CStr::from_ptr(loader) }
            .to_bytes()
            .windows(3)
            .any(|part| part.eq_ignore_ascii_case(b"svg"));
    if !is_svg {
        return Ok(None);
    }
    let scale = height.max(1.0) / natural_height.max(1) as f64;
    write_operation(VipsImage::new_memory()?, |out| unsafe {
        vips_sys::vips_svgload_buffer(
            bytes.as_ptr().cast_mut().cast(),
            bytes.len(),
            out,
            c"scale".as_ptr(),
            scale,
            ptr::null::<i8>(),
        )
    })
    .map(Some)
}

/// 将 vips 图像求值为连续像素内存，并在 libvips 失败时返回错误。
pub fn image_write_to_memory(image: &VipsImage) -> Result<Vec<u8>> {
    ensure_vips();
    image.write_to_memory().map_err(Into::into)
}

pub fn image_metadata_string(image: &VipsImage, name: &str) -> Result<String> {
    ensure_vips();
    let name = std::ffi::CString::new(name)?;
    let mut value = ptr::null_mut();
    let status =
        unsafe { vips_sys::vips_image_get_as_string(image.as_ptr(), name.as_ptr(), &mut value) };
    vips::code_to_result(status)?;
    anyhow::ensure!(!value.is_null(), "libvips 未返回元数据值");
    let text = unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() };
    unsafe { vips_sys::g_free(value.cast()) };
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_uchar_and_constant_like_reject_invalid_slices() {
        ensure_vips();
        let black = vips::black(4, 4).unwrap();
        assert!(linear_uchar(&black, &[], &[]).is_err());
        assert!(linear_uchar(&black, &[1.0], &[0.0, 1.0]).is_err());
        assert!(constant_like(&black, &[]).is_err());
    }
}
