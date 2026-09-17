//! 黑白底图配合彩色恢复 gain map。
//!
//! 新的 gain map 记录从黑白 SDR 底图恢复到原始 RGB 的逐通道增益。输入已有 gain map
//! 时，会将其 HDR 高光叠加在原始 SDR 亮度之上再重新编码。

use anyhow::Result;
use libvips::{VipsImage, ops};
use std::path::Path;

use crate::{photo::Photo, process::gain_map};

/// 读取一张图片，将底图转换为黑白，并生成可恢复完整原图颜色的 gain map。
///
/// 返回的图片是 libvips 图像；如需编码到文件，调用方应沿用 [`Photo::save_image`] 或
/// 自己选择合适的编码器。普通照片也能使用；若原图已有 gain map，会先解码后重编码。
pub fn load_black_and_white_with_colour_gainmap(path: &Path) -> Result<VipsImage> {
    let base = Photo::load_base_image(path)?;
    let source_sdr = ops::s_rgb2sc_rgb(&base)?;
    let target_hdr = if gain_map::get_gainmap(&base).is_some() {
        // 有些原 Ultra HDR 的 HDR intent 会比它的 SDR 底图更暗。使用 SDR 彩色图作为
        // 下限，只叠加原 gain map 中确实更亮的部分，避免彩色预览整体被压暗。
        let source_hdr = ops::uhdr2sc_rgb(&base)?;
        ops::maxpair(&source_sdr, &source_hdr)?
    } else {
        source_sdr
    };

    // JPEG 的 SDR 底图保持三个相同的 RGB 通道，旧设备也会将它显示为黑白图。
    let black_and_white = ops::colourspace(&base, ops::Interpretation::BW)?;
    let mut black_and_white = ops::colourspace(&black_and_white, ops::Interpretation::Srgb)?;
    let base_sdr = ops::s_rgb2sc_rgb(&black_and_white)?;
    let (colour_gainmap, min_boost, max_boost) = encode_rgb_gainmap(&base_sdr, &target_hdr)?;
    gain_map::set_gainmap(&mut black_and_white, &colour_gainmap);
    gain_map::set_scale_factor(&mut black_and_white, 1.0);
    gain_map::set_rgb_gainmap_metadata(&mut black_and_white, min_boost, max_boost);

    Ok(black_and_white)
}

/// 编码逐 RGB 通道增益，并返回与编码匹配的最小、最大 boost。
fn encode_rgb_gainmap(
    base_sdr: &VipsImage,
    target_hdr: &VipsImage,
) -> Result<(VipsImage, [f64; 3], [f64; 3])> {
    const OFFSET: f64 = 1.0 / 64.0;
    const MIN_RANGE: f64 = 1.001;

    let source = ops::linear(base_sdr, &mut [1.0; 3], &mut [OFFSET; 3])?;
    let target = ops::linear(target_hdr, &mut [1.0; 3], &mut [OFFSET; 3])?;
    let gain = ops::divide(&target, &source)?;
    let mut min_boost = [0.0; 3];
    let mut max_boost = [0.0; 3];
    let mut encoded_bands = Vec::with_capacity(3);

    for band in 0..3 {
        let gain_band = ops::extract_band(&gain, band)?;
        let min = ops::min(&gain_band)?.max(f64::MIN_POSITIVE);
        let max = ops::max(&gain_band)?.max(min * MIN_RANGE);
        min_boost[band as usize] = min;
        max_boost[band as usize] = max;
        let log_gain = ops::math(&gain_band, ops::OperationMath::Log)?;
        let range = (max / min).ln();
        encoded_bands.push(ops::linear_with_opts(
            &log_gain,
            &mut [255.0 / range],
            &mut [-min.ln() * 255.0 / range],
            &ops::LinearOptions { uchar: true },
        )?);
    }

    let gainmap = ops::bandjoin(&mut encoded_bands)?;
    // 处理链会把输入 Ultra HDR 的 metadata 传递给输出图。重新从像素内存创建图像，
    // 使 gain map 成为不带 ICC、EXIF 或嵌套 gain map 的纯编码载体。
    let gainmap = VipsImage::new_from_memory_copy(
        &gainmap.image_write_to_memory(),
        gainmap.get_width(),
        gainmap.get_height(),
        gainmap.get_bands(),
        ops::BandFormat::Uchar,
    )?;
    let gainmap = ops::copy_with_opts(
        &gainmap,
        &ops::CopyOptions {
            width: gainmap.get_width(),
            height: gainmap.get_height(),
            bands: gainmap.get_bands(),
            interpretation: ops::Interpretation::Srgb,
            ..Default::default()
        },
    )?;
    Ok((gainmap, min_boost, max_boost))
}
