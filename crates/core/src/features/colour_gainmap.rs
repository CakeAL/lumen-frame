//! 黑白底图配合彩色恢复 gain map。
//!
//! 新的 gain map 记录从黑白 SDR 底图恢复到原始 RGB 的逐通道增益。输入已有 gain map
//! 时，会将其 HDR 高光叠加在原始 SDR 亮度之上再重新编码。

use anyhow::Result;
use std::path::Path;
use vips::{VipsBandFormat, VipsInterpretation, VipsOperationMath};

use crate::{
    gainmap as gain_map,
    media::{
        load_base_image,
        vips::{VipsImage, image_op},
    },
    rotation::{Rotation, apply_to_output},
};

/// 读取一张图片，将底图转换为黑白，并生成可恢复完整原图颜色的 gain map。
///
/// 返回的图片是 libvips 图像；调用方自行选择编码器。普通照片也能使用；若原图已有
/// gain map，会先解码后重编码。
pub fn load_black_and_white_with_colour_gainmap(
    path: &Path,
    rotation: Rotation,
) -> Result<VipsImage> {
    let base = load_base_image(path)?;
    let source_sdr = image_op(|out| unsafe {
        vips_sys::vips_sRGB2scRGB(base.as_ptr(), out, std::ptr::null::<i8>())
    })?;
    let target_hdr = if gain_map::get_gainmap(&base).is_some() {
        // 有些原 Ultra HDR 的 HDR intent 会比它的 SDR 底图更暗。使用 SDR 彩色图作为
        // 下限，只叠加原 gain map 中确实更亮的部分，避免彩色预览整体被压暗。
        let source_hdr = image_op(|out| unsafe {
            vips_sys::vips_uhdr2scRGB(base.as_ptr(), out, std::ptr::null::<i8>())
        })?;
        image_op(|out| unsafe {
            vips_sys::vips_maxpair(
                source_sdr.as_ptr(),
                source_hdr.as_ptr(),
                out,
                std::ptr::null::<i8>(),
            )
        })?
    } else {
        source_sdr
    };
    // JPEG 的 SDR 底图保持三个相同的 RGB 通道，旧设备也会将它显示为黑白图。
    let black_and_white = image_op(|out| unsafe {
        vips_sys::vips_colourspace(
            base.as_ptr(),
            out,
            VipsInterpretation::VIPS_INTERPRETATION_B_W,
            std::ptr::null::<i8>(),
        )
    })?;
    let mut black_and_white = image_op(|out| unsafe {
        vips_sys::vips_colourspace(
            black_and_white.as_ptr(),
            out,
            VipsInterpretation::VIPS_INTERPRETATION_sRGB,
            std::ptr::null::<i8>(),
        )
    })?;
    let base_sdr = image_op(|out| unsafe {
        vips_sys::vips_sRGB2scRGB(black_and_white.as_ptr(), out, std::ptr::null::<i8>())
    })?;
    let (colour_gainmap, min_boost, max_boost) = encode_rgb_gainmap(&base_sdr, &target_hdr)?;
    gain_map::set_gainmap(&mut black_and_white, &colour_gainmap);
    gain_map::set_scale_factor(&mut black_and_white, 1.0);
    gain_map::set_rgb_gainmap_metadata(&mut black_and_white, min_boost, max_boost);

    // 先完整生成黑白底图与彩色 gain map，再把二者作为一张成片统一旋转。
    apply_to_output(&black_and_white, rotation)
}

/// 编码逐 RGB 通道增益，并返回与编码匹配的最小、最大 boost。
fn encode_rgb_gainmap(
    base_sdr: &VipsImage,
    target_hdr: &VipsImage,
) -> Result<(VipsImage, [f64; 3], [f64; 3])> {
    const OFFSET: f64 = 1.0 / 64.0;
    const MIN_RANGE: f64 = 1.001;

    let a = [1.0; 3];
    let b = [OFFSET; 3];
    let source = image_op(|out| unsafe {
        vips_sys::vips_linear(
            base_sdr.as_ptr(),
            out,
            a.as_ptr(),
            b.as_ptr(),
            3,
            std::ptr::null::<i8>(),
        )
    })?;
    let target = image_op(|out| unsafe {
        vips_sys::vips_linear(
            target_hdr.as_ptr(),
            out,
            a.as_ptr(),
            b.as_ptr(),
            3,
            std::ptr::null::<i8>(),
        )
    })?;
    let gain = image_op(|out| unsafe {
        vips_sys::vips_divide(
            target.as_ptr(),
            source.as_ptr(),
            out,
            std::ptr::null::<i8>(),
        )
    })?;
    let mut min_boost = [0.0; 3];
    let mut max_boost = [0.0; 3];
    let mut encoded_bands = Vec::with_capacity(3);

    for band in 0..3 {
        let gain_band = image_op(|out| unsafe {
            vips_sys::vips_extract_band(gain.as_ptr(), out, band, std::ptr::null::<i8>())
        })?;
        let mut min = 0.0;
        vips::code_to_result(unsafe {
            vips_sys::vips_min(gain_band.as_ptr(), &mut min, std::ptr::null::<i8>())
        })?;
        let min = min.max(f64::MIN_POSITIVE);
        let mut max = 0.0;
        vips::code_to_result(unsafe {
            vips_sys::vips_max(gain_band.as_ptr(), &mut max, std::ptr::null::<i8>())
        })?;
        let max = max.max(min * MIN_RANGE);
        min_boost[band as usize] = min;
        max_boost[band as usize] = max;
        let log_gain = image_op(|out| unsafe {
            vips_sys::vips_math(
                gain_band.as_ptr(),
                out,
                VipsOperationMath::VIPS_OPERATION_MATH_LOG,
                std::ptr::null::<i8>(),
            )
        })?;
        let range = (max / min).ln();
        let a = [255.0 / range];
        let b = [-min.ln() * 255.0 / range];
        encoded_bands.push(image_op(|out| unsafe {
            vips_sys::vips_linear(
                log_gain.as_ptr(),
                out,
                a.as_ptr(),
                b.as_ptr(),
                1,
                c"uchar".as_ptr(),
                1_i32,
                std::ptr::null::<i8>(),
            )
        })?);
    }

    let mut band_ptrs: Vec<_> = encoded_bands.iter().map(VipsImage::as_ptr).collect();
    let gainmap = image_op(|out| unsafe {
        vips_sys::vips_bandjoin(
            band_ptrs.as_mut_ptr(),
            out,
            band_ptrs.len() as i32,
            std::ptr::null::<i8>(),
        )
    })?;
    // 处理链会把输入 Ultra HDR 的 metadata 传递给输出图。重新从像素内存创建图像，
    // 使 gain map 成为不带 ICC、EXIF 或嵌套 gain map 的纯编码载体。
    let gainmap = VipsImage::from_memory(
        gainmap.write_to_memory()?,
        gainmap.width(),
        gainmap.height(),
        gainmap.bands() as u8,
        VipsBandFormat::VIPS_FORMAT_UCHAR,
    )?;
    let gainmap = image_op(|out| unsafe {
        vips_sys::vips_copy(
            gainmap.as_ptr(),
            out,
            c"width".as_ptr(),
            gainmap.width() as i32,
            c"height".as_ptr(),
            gainmap.height() as i32,
            c"bands".as_ptr(),
            gainmap.bands() as i32,
            c"interpretation".as_ptr(),
            VipsInterpretation::VIPS_INTERPRETATION_sRGB,
            std::ptr::null::<i8>(),
        )
    })?;
    Ok((gainmap, min_boost, max_boost))
}
