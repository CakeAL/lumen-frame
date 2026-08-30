//! 生成带边框与拍摄参数水印的照片。
//!
//! 流程：载入 JPEG → 按边框比例扩展画布 → 将原图居中放置 → 从 Exif 中提取
//! 拍摄参数（ISO / 快门 / 光圈 / 焦距 / 等效 35mm 焦距）并渲染到底部。

use std::path::PathBuf;

use anyhow::{Context, Result};
use libvips::ops::{self, BlendMode, CompassDirection, Extend, Interpretation};
use libvips::VipsApp;
use nom_exif::{EntryValue, Exif, ExifTag, MediaParser};

use crate::parse::dump_exif;

/// 水印生成参数。
#[derive(Debug, Clone)]
pub struct WatermarkParams {
    /// 源照片路径。
    pub input_path: PathBuf,
    /// 输出图片路径。
    pub output_path: PathBuf,
    /// 边框比例，同时作用于宽高。例如 `0.05` 表示输出尺寸 = 原尺寸 * `1.05`。
    pub border_ratio: f64,
    /// 背景颜色 (R, G, B)，默认纯白。
    pub background: [u8; 3],
    /// 信息文字颜色 (R, G, B)，默认深灰。
    pub text_color: [u8; 3],
    /// 信息文字字体（Pango 描述，例如 `"sans 48"`）；`None` 时按边框高度自动估算。
    pub font: Option<String>,
    /// 文字渲染 DPI，默认 72（此时 Pango 字号 1pt ≈ 1px）。
    pub dpi: i32,
    /// 输出 JPEG 质量 1-100，默认 95。
    pub quality: i32,
}

impl WatermarkParams {
    /// 使用默认参数创建实例：边框 5%、纯白背景、深灰文字。
    pub fn new(input_path: impl Into<PathBuf>, output_path: impl Into<PathBuf>) -> Self {
        Self {
            input_path: input_path.into(),
            output_path: output_path.into(),
            border_ratio: 0.05,
            background: [255, 255, 255],
            text_color: [60, 60, 60],
            font: None,
            dpi: 72,
            quality: 95,
        }
    }
}

/// 从 Exif 中提取、用于水印的拍摄参数（每一项都是可选的）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CaptureInfo {
    /// 感光度，如 `"ISO 100"`。
    pub iso: Option<String>,
    /// 快门速度，如 `"1/250s"`。
    pub shutter_speed: Option<String>,
    /// 光圈，如 `"f/1.8"`。
    pub aperture: Option<String>,
    /// 焦距，如 `"50mm"`。
    pub focal_length: Option<String>,
    /// 等效 35mm 焦距，如 `"35mm: 75mm"`。
    pub focal_length_35mm: Option<String>,
}

impl CaptureInfo {
    /// 渲染为底部水印文字，缺失的项自动跳过；全部缺失时返回 `None`。
    pub fn to_caption(&self) -> Option<String> {
        let parts: Vec<&str> = [
            self.iso.as_deref(),
            self.shutter_speed.as_deref(),
            self.aperture.as_deref(),
            self.focal_length.as_deref(),
            self.focal_length_35mm.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("  "))
        }
    }
}

/// 从 [`Exif`] 中提取拍摄参数。
pub fn extract_capture_info(exif: &Exif) -> CaptureInfo {
    let get = |tag| find_value(exif, tag);

    CaptureInfo {
        iso: get(ExifTag::ISOSpeedRatings).and_then(format_iso),
        shutter_speed: get(ExifTag::ExposureTime).and_then(format_shutter_speed),
        aperture: get(ExifTag::FNumber).and_then(format_aperture),
        focal_length: get(ExifTag::FocalLength).and_then(format_focal_length),
        focal_length_35mm: get(ExifTag::FocalLengthIn35mmFilm).and_then(format_focal_length_35mm),
    }
}

/// 在所有 IFD 中查找指定 tag 的首个条目（拍摄参数通常位于 Exif 子 IFD 中）。
fn find_value<'a>(exif: &'a Exif, tag: ExifTag) -> Option<&'a EntryValue> {
    exif.iter()
        .find(|e| e.tag.tag() == Some(tag))
        .map(|e| e.value)
}

fn format_iso(value: &EntryValue) -> Option<String> {
    match value {
        // 部分相机将 ISO 存为数组（如 [100]）。
        EntryValue::U16Array(v) => v.first().map(|n| format!("ISO {n}")),
        EntryValue::U32Array(v) => v.first().map(|n| format!("ISO {n}")),
        _ => value.try_as_integer().map(|n| format!("ISO {n}")),
    }
}

fn format_shutter_speed(value: &EntryValue) -> Option<String> {
    if let Some(r) = value.as_urational() {
        let n = r.numerator();
        let d = r.denominator();
        if d == 0 {
            return None;
        }
        if n == 1 {
            return Some(format!("1/{d}s"));
        }
        return Some(format!("{}s", trim_f64(r.to_f64()?)));
    }
    value.try_as_float().map(|s| format!("{}s", trim_f64(s)))
}

fn format_aperture(value: &EntryValue) -> Option<String> {
    let f = value.as_urational()?.to_f64()?;
    Some(format!("f/{}", trim_f64(f)))
}

fn format_focal_length(value: &EntryValue) -> Option<String> {
    let f = value.as_urational()?.to_f64()?;
    Some(format!("{}mm", trim_f64(f)))
}

fn format_focal_length_35mm(value: &EntryValue) -> Option<String> {
    value.try_as_integer().map(|n| format!("35mm: {n}mm"))
}

/// 将浮点数格式化为最多两位小数，并去掉多余的末尾 `0`。
fn trim_f64(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 生成带边框与水印的照片。
///
/// Exif 读取失败（例如照片本身没有 Exif）不会导致整体失败，仅会跳过水印文字。
pub async fn generate_watermark(params: &WatermarkParams) -> Result<()> {
    // 初始化 libvips。
    let _app = VipsApp::default("lumen-frame").context("failed to init libvips")?;

    // 载入并按 EXIF 方向摆正原图。
    let img = ops::jpegload_with_opts(
        &params.input_path.to_string_lossy(),
        &ops::JpegloadOptions {
            autorotate: true,
            ..Default::default()
        },
    )
    .context("failed to load image")?;

    let img_w = img.get_width();
    let img_h = img.get_height();

    // 计算带边框画布尺寸（宽高分别乘以 1 + 比例）。
    let canvas_w = (img_w as f64 * (1.0 + params.border_ratio)).round() as i32;
    let canvas_h = (img_h as f64 * (1.0 + params.border_ratio)).round() as i32;

    // 把原图居中放到画布上。
    let [br, bg, bb] = params.background;
    let gravity_opts = ops::GravityOptions {
        extend: Extend::Background,
        background: vec![br as f64, bg as f64, bb as f64],
    };
    let mut canvas = ops::gravity_with_opts(
        &img,
        CompassDirection::Centre,
        canvas_w,
        canvas_h,
        &gravity_opts,
    )
    .context("failed to place image")?;

    // 从 parse 模块读取 Exif（尽力而为），提取并渲染底部水印。
    let mut parser = MediaParser::new();
    let caption = match dump_exif(&mut parser, &params.input_path).await {
        Ok(Some(exif)) => extract_capture_info(&exif).to_caption(),
        _ => None,
    };

    if let Some(caption) = caption {
        let text = render_caption(&caption, params, img_h, canvas_h)?;
        let tw = text.get_width();
        let th = text.get_height();

        // 水平居中；垂直方向居中于底部边框区域内。
        let h_extra = canvas_h - img_h;
        let border_top = h_extra / 2;
        let border_bottom = h_extra - border_top;
        let x = (canvas_w - tw) / 2;
        let y = img_h + border_top + ((border_bottom - th) / 2).max(0);

        let composite_opts = ops::Composite2Options {
            x,
            y,
            compositing_space: Interpretation::Srgb,
            premultiplied: false,
        };
        let composed = ops::composite2_with_opts(&canvas, &text, BlendMode::Over, &composite_opts)
            .context("failed to composite caption")?;

        // composite2 之后带上了 alpha 通道，写出 JPEG 前先压平为 RGB。
        let flatten_opts = ops::FlattenOptions {
            background: vec![br as f64, bg as f64, bb as f64],
            ..Default::default()
        };
        canvas = ops::flatten_with_opts(&composed, &flatten_opts)
            .context("failed to flatten image")?;
    }

    // 写出 JPEG。
    ops::jpegsave_with_opts(
        &canvas,
        &params.output_path.to_string_lossy(),
        &ops::JpegsaveOptions {
            q: params.quality,
            ..Default::default()
        },
    )
    .context("failed to save image")?;

    Ok(())
}

/// 渲染底部水印文字：白色文字 → 目标颜色（保持 alpha 不变）。
fn render_caption(
    caption: &str,
    params: &WatermarkParams,
    img_h: i32,
    canvas_h: i32,
) -> Result<libvips::VipsImage> {
    // 自动估算字号：约为底部边框高度的 55%，并限制在合理范围内。
    let border = ((canvas_h - img_h) / 2).max(1);
    let auto_size = ((border as f64) * 0.55).round().clamp(10.0, 400.0) as i32;
    let font = params
        .font
        .clone()
        .unwrap_or_else(|| format!("sans {auto_size}"));

    let text_opts = ops::TextOptions {
        font: Some(font),
        dpi: params.dpi,
        rgba: true,
        ..Default::default()
    };
    let white = ops::text_with_opts(caption, &text_opts).context("failed to render text")?;

    // 白色 (255) * (color / 255) = color；alpha 系数为 1，保持不变。
    let [tr, tg, tb] = params.text_color;
    let mut a = [
        tr as f64 / 255.0,
        tg as f64 / 255.0,
        tb as f64 / 255.0,
        1.0,
    ];
    let mut b = [0.0, 0.0, 0.0, 0.0];
    let linear_opts = ops::LinearOptions { uchar: true };
    ops::linear_with_opts(&white, &mut a, &mut b, &linear_opts).context("failed to color text")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom_exif::{EntryValue, URational};

    #[test]
    fn shutter_speed_is_fraction_when_numerator_is_one() {
        let v = EntryValue::URational(URational::new(1, 250));
        assert_eq!(format_shutter_speed(&v).as_deref(), Some("1/250s"));
    }

    #[test]
    fn shutter_speed_is_decimal_otherwise() {
        let v = EntryValue::URational(URational::new(3, 10));
        assert_eq!(format_shutter_speed(&v).as_deref(), Some("0.3s"));
    }

    #[test]
    fn aperture_and_focal_length_format() {
        assert_eq!(
            format_aperture(&EntryValue::URational(URational::new(18, 10))).as_deref(),
            Some("f/1.8")
        );
        assert_eq!(
            format_focal_length(&EntryValue::URational(URational::new(50, 1))).as_deref(),
            Some("50mm")
        );
    }

    #[test]
    fn iso_and_35mm_format() {
        assert_eq!(format_iso(&EntryValue::U16(100)).as_deref(), Some("ISO 100"));
        assert_eq!(
            format_focal_length_35mm(&EntryValue::U16(75)).as_deref(),
            Some("35mm: 75mm")
        );
    }

    #[test]
    fn caption_skips_missing_fields() {
        let info = CaptureInfo {
            iso: Some("ISO 100".into()),
            aperture: Some("f/1.8".into()),
            ..Default::default()
        };
        assert_eq!(info.to_caption().as_deref(), Some("ISO 100  f/1.8"));
    }

    #[test]
    fn caption_is_none_when_everything_is_missing() {
        assert_eq!(CaptureInfo::default().to_caption(), None);
    }

    /// 端到端测试：读取真实照片，生成带边框与拍摄参数水印的输出。
    #[tokio::test]
    async fn generate_watermark_to_file() {
        let input = "/Users/cakeal/Downloads/DSC_7379.jpg";
        let output = "/Users/cakeal/Downloads/DSC_7379_watermark.jpg";

        let params = WatermarkParams::new(input, output);
        generate_watermark(&params).await.expect("generate watermark");

        let out = std::path::Path::new(output);
        assert!(out.exists(), "output image not written: {output}");
        assert!(out.metadata().unwrap().len() > 0, "output image is empty");
    }
}
