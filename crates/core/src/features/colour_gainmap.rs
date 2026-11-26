//! 黑白底图配合彩色恢复 gain map。
//!
//! 新的 gain map 记录从黑白 SDR 底图恢复到原始 RGB 的逐通道增益。输入已有 gain map
//! 时，会将其 HDR 高光叠加在原始 SDR 亮度之上再重新编码。

use crate::media::vips::{linear_uchar, max_pair, with_interpretation};
use anyhow::Result;
use std::path::Path;
use vips::{VipsBandFormat, VipsInterpretation};

use crate::{
    gainmap as gain_map,
    media::{load_base_image, vips::VipsImage},
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
    let source_sdr = base.colourspace(VipsInterpretation::VIPS_INTERPRETATION_scRGB)?;
    let target_hdr = if gain_map::get_gainmap(&base).is_some() {
        // 有些原 Ultra HDR 的 HDR intent 会比它的 SDR 底图更暗。使用 SDR 彩色图作为
        // 下限，只叠加原 gain map 中确实更亮的部分，避免彩色预览整体被压暗。
        let source_hdr = base.uhdr2scrgb()?;
        max_pair(&source_sdr, &source_hdr)?
    } else {
        source_sdr
    };
    // JPEG 的 SDR 底图保持三个相同的 RGB 通道，旧设备也会将它显示为黑白图。
    let black_and_white = base.colourspace(VipsInterpretation::VIPS_INTERPRETATION_B_W)?;
    let mut black_and_white =
        black_and_white.colourspace(VipsInterpretation::VIPS_INTERPRETATION_sRGB)?;
    let base_sdr = black_and_white.colourspace(VipsInterpretation::VIPS_INTERPRETATION_scRGB)?;
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
    let source = base_sdr.linear(&a, &b)?;
    let target = target_hdr.linear(&a, &b)?;
    let gain = target.divide(&source)?;
    let mut min_boost = [0.0; 3];
    let mut max_boost = [0.0; 3];
    let mut encoded_bands = Vec::with_capacity(3);

    for band in 0..3 {
        let gain_band = gain.extract_band(band)?;
        let min = gain_band.min_value()?.max(f64::MIN_POSITIVE);
        let max = gain_band.max_value()?.max(min * MIN_RANGE);
        min_boost[band as usize] = min;
        max_boost[band as usize] = max;
        let log_gain = gain_band.ln()?;
        let range = (max / min).ln();
        let a = [255.0 / range];
        let b = [-min.ln() * 255.0 / range];
        encoded_bands.push(linear_uchar(&log_gain, &a, &b)?);
    }

    let [red, green, blue] = encoded_bands.as_slice() else {
        unreachable!("RGB gain map has three bands");
    };
    let gainmap = red.bandjoin2(green)?.bandjoin2(blue)?;
    // 处理链会把输入 Ultra HDR 的 metadata 传递给输出图。重新从像素内存创建图像，
    // 使 gain map 成为不带 ICC、EXIF 或嵌套 gain map 的纯编码载体。
    let gainmap = VipsImage::from_memory(
        gainmap.write_to_memory()?,
        gainmap.width(),
        gainmap.height(),
        gainmap.bands() as u8,
        VipsBandFormat::VIPS_FORMAT_UCHAR,
    )?;
    let gainmap = with_interpretation(&gainmap, VipsInterpretation::VIPS_INTERPRETATION_sRGB)?;
    Ok((gainmap, min_boost, max_boost))
}
