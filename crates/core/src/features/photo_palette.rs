//! 可复用的照片色卡提取。只分析缩小后的 SDR 颜色，不修改原图或 HDR 元数据。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

use anyhow::{Context as _, Result, ensure};
use auto_palette::{ImageData, Palette};
use vips::{VipsBandFormat, VipsInterpretation, VipsSize};

use crate::media::{
    ensure_vips, image_write_to_memory,
    vips::{VipsImage, image_op},
};
use crate::watermark::{BackgroundGradient, PaletteTheme, WatermarkParams};

#[derive(PartialEq)]
struct SourceStamp {
    len: u64,
    modified: Option<SystemTime>,
}

type CachedPalettes = HashMap<PathBuf, (SourceStamp, Arc<Palette<f64>>)>;

fn palette_cache() -> &'static Mutex<CachedPalettes> {
    static CACHE: OnceLock<Mutex<CachedPalettes>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaletteSwatch {
    rgb: [u8; 3],
    ratio: f64,
}

impl PaletteSwatch {
    pub fn rgb(&self) -> [u8; 3] {
        self.rgb
    }
    pub fn ratio(&self) -> f64 {
        self.ratio
    }
}

/// 提取最多 `count` 个代表色。会阻塞，界面应在后台线程调用。
pub fn extract(path: &Path, count: usize) -> Result<Vec<PaletteSwatch>> {
    extract_with_theme(path, count, PaletteTheme::Default)
}

pub fn extract_with_theme(
    path: &Path,
    count: usize,
    theme: PaletteTheme,
) -> Result<Vec<PaletteSwatch>> {
    ensure!((1..=16).contains(&count), "色卡数量必须在 1 至 16 之间");
    let metadata = std::fs::metadata(path).context("读取取色照片信息失败")?;
    let stamp = SourceStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };
    let cached = palette_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .filter(|(old, _)| *old == stamp)
        .map(|(_, palette)| palette.clone());
    let palette = if let Some(palette) = cached {
        palette
    } else {
        let palette = Arc::new(load_palette(path)?);
        // 只缓存纯 Rust 的聚类结果，不跨线程保留 libvips 图像。限制总量避免大队列常驻内存。
        let mut cache = palette_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), (stamp, palette.clone()));
        palette
    };
    select_swatches(&palette, count, theme)
}

fn load_palette(path: &Path) -> Result<Palette<f64>> {
    ensure_vips();
    // thumbnail 使用解码器缩小能力，并按 EXIF 摆正；统一经过 libvips 支持原有格式。
    let source = VipsImage::from_file(path).context("读取照片失败")?;
    let image = source.thumbnail(192, 192, VipsSize::VIPS_SIZE_DOWN)?;
    let image = image_op(|out| unsafe {
        vips_sys::vips_colourspace(
            image.as_ptr(),
            out,
            VipsInterpretation::VIPS_INTERPRETATION_sRGB,
            std::ptr::null::<i8>(),
        )
    })?;
    let image = image_op(|out| unsafe {
        vips_sys::vips_cast(
            image.as_ptr(),
            out,
            VipsBandFormat::VIPS_FORMAT_UCHAR,
            std::ptr::null::<i8>(),
        )
    })?;
    let pixels = image_write_to_memory(&image)?;
    let rgba = match image.bands() {
        3 => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        4 => pixels,
        bands => anyhow::bail!("取色不支持 {bands} 个颜色通道，请转换为 RGB 照片"),
    };
    let image = ImageData::new(image.width(), image.height(), &rgba)?;
    Palette::extract(&image).context("提取照片代表色失败")
}

fn select_swatches(
    palette: &Palette<f64>,
    count: usize,
    theme: PaletteTheme,
) -> Result<Vec<PaletteSwatch>> {
    #[allow(deprecated)]
    let theme = match theme {
        PaletteTheme::Default => None,
        PaletteTheme::Basic => Some(auto_palette::Theme::Basic),
        PaletteTheme::Colorful => Some(auto_palette::Theme::Colorful),
        PaletteTheme::Vivid => Some(auto_palette::Theme::Vivid),
        PaletteTheme::Muted => Some(auto_palette::Theme::Muted),
        PaletteTheme::Light => Some(auto_palette::Theme::Light),
        PaletteTheme::Dark => Some(auto_palette::Theme::Dark),
    };
    // 某些单色照片没有符合主题的候选色，回退到原图代表色，避免套用设置后无法导出。
    let swatches = theme
        .and_then(|theme| palette.find_swatches_with_theme(count, theme).ok())
        .filter(|colors| !colors.is_empty())
        .map(Ok)
        .unwrap_or_else(|| palette.find_swatches(count))
        .context("选择照片色卡失败")?;
    let mut result = Vec::new();
    for swatch in swatches {
        let color = swatch.color().to_rgb();
        let rgb = [color.r, color.g, color.b];
        if !result.iter().any(|s: &PaletteSwatch| s.rgb == rgb) {
            result.push(PaletteSwatch {
                rgb,
                ratio: swatch.ratio(),
            });
        }
    }
    ensure!(
        !result.is_empty(),
        "照片没有可用颜色，请选择非透明照片后重试"
    );
    // 面积相同时仍保持确定的序号，避免界面和导出分别选色时交换相同占比的色标。
    result.sort_by(|a, b| b.ratio.total_cmp(&a.ratio).then(a.rgb.cmp(&b.rgb)));
    Ok(result)
}

/// 按同一组代表色序号在目标照片上求值；手动色标保留原值。
pub fn gradient_colors(params: &WatermarkParams, swatches: &[PaletteSwatch]) -> Vec<[u8; 3]> {
    let mut colors = params.gradient.colors().to_vec();
    if swatches.is_empty() {
        return colors;
    }
    let slots = params.gradient_palette.slots(colors.len());
    for (color, slot) in colors.iter_mut().zip(slots) {
        if let Some(rank) = slot {
            *color = swatches[rank.min(swatches.len() - 1)].rgb();
        }
    }
    colors
}

/// 预览和导出都必须在后台按原照片解析取色规则，不使用另一张照片的已求值颜色。
pub fn resolve(path: &Path, params: &WatermarkParams) -> Result<WatermarkParams> {
    let mut resolved = params.clone();
    let gradient_palette = params.gradient_background && params.gradient_palette.uses_palette();
    let border_palette =
        params.photo_border.enabled() && params.photo_border.palette_rank().is_some();
    if gradient_palette || border_palette {
        let swatches = extract_with_theme(path, 6, params.gradient_palette.theme())?;
        if gradient_palette {
            resolved.gradient = BackgroundGradient::new(
                params.gradient.angle(),
                gradient_colors(params, &swatches),
            )?;
        }
        if border_palette {
            let rank = params
                .photo_border
                .palette_rank()
                .unwrap()
                .min(swatches.len() - 1);
            resolved.photo_border.resolve_color(swatches[rank].rgb());
        }
    }
    if params.palette_card.enabled() {
        let swatches = extract_with_theme(path, 6, params.palette_card.theme())?;
        resolved
            .palette_card
            .resolve(swatches.iter().map(|s| (s.rgb(), s.ratio())).collect());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_real_sdr_and_hdr_photos_without_modifying_them() {
        for path in [
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/DSC_4587.jpg"
            ),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/ultra_hdr.jpg"
            ),
        ] {
            let before = std::fs::read(path).unwrap();
            let swatches = extract(Path::new(path), 6).unwrap();
            assert!((1..=6).contains(&swatches.len()));
            assert!(swatches.iter().all(|s| s.ratio() > 0.0 && s.ratio() <= 1.0));
            assert_eq!(before, std::fs::read(path).unwrap());
        }
    }

    #[test]
    fn all_themes_are_available_and_gradient_sources_follow_the_target_photo() {
        use crate::watermark::GradientPalette;
        let themes = [
            PaletteTheme::Default,
            PaletteTheme::Basic,
            PaletteTheme::Colorful,
            PaletteTheme::Vivid,
            PaletteTheme::Muted,
            PaletteTheme::Light,
            PaletteTheme::Dark,
        ];
        let mut first_colors = None;
        for theme in themes {
            let first = extract_with_theme(
                Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../test_images/DSC_4587.jpg"
                )),
                6,
                theme,
            )
            .unwrap();
            let second = extract_with_theme(
                Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../test_images/ultra_hdr.jpg"
                )),
                6,
                theme,
            )
            .unwrap();
            assert!(!first.is_empty() && !second.is_empty());
            let params = WatermarkParams {
                gradient_background: true,
                gradient: BackgroundGradient::new(
                    135.0,
                    vec![[1, 2, 3], first[0].rgb(), first.last().unwrap().rgb()],
                )
                .unwrap(),
                gradient_palette: GradientPalette::new(theme, vec![None, Some(0), Some(999)]),
                ..Default::default()
            };
            let resolved = resolve(
                Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../test_images/ultra_hdr.jpg"
                )),
                &params,
            )
            .unwrap();
            assert_eq!(
                resolved.gradient.colors(),
                &[[1, 2, 3], second[0].rgb(), second.last().unwrap().rgb()]
            );
            assert_eq!(resolved.gradient_palette, params.gradient_palette);
            if theme == PaletteTheme::Default {
                first_colors = Some((first[0].rgb(), second[0].rgb()));
            }
        }
        let (first, second) = first_colors.unwrap();
        assert_ne!(first, second, "夹具应有不同主色，以验证不是复制原图颜色");
    }
}
