use anyhow::{Context, Result};
use libvips::{VipsApp, ops};
use nom_exif::{EntryValue, Exif, ExifDateTime, ExifTag, read_exif_async};
use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

use crate::params::WatermarkParams;

#[derive(Debug, Clone)]
pub struct Photo {
    pub path: PathBuf,
    pub is_motion_photo: bool,
    pub is_ultra_hdr_photo: bool,
    pub exif: Option<ExifInfo>,
}

impl Photo {
    pub async fn new(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            is_motion_photo: false,
            is_ultra_hdr_photo: false,
            exif: Some(ExifInfo::new(path.as_ref()).await?),
        })
    }

    pub async fn generate_watermark(&self, params: &WatermarkParams) -> Result<()> {
        let _app = VipsApp::default("luman-frame").context("failed to init libvips")?;

        // 摆正原图
        let img = ops::jpegload_with_opts(
            &self.path.to_string_lossy(),
            &ops::JpegloadOptions {
                autorotate: true,
                ..Default::default()
            },
        )
        .context("failed to load image")?;

        // 计算水印照片的图片尺寸
        let (img_w, img_h) = (img.get_width(), img.get_height());
        let canvas_h = (img_h as f64 * (1.0 + params.border_ratio.0)).round() as i32;
        let canvas_w = (img_w as f64
            * (1.0
                + if params.border_equal {
                    params.border_ratio.0
                } else {
                    params.border_ratio.1
                }))
        .round() as i32;

        Ok(())
    }
}

/// 光圈，快门速度可能是小数或者分数
#[derive(Debug, Clone)]
pub enum Rational {
    Fraction(u32, u32),
    Float(f64),
}

impl Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format_value = |v: f64| {
            format!("{v:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        match self {
            Rational::Fraction(n, d) => {
                let v = *n as f64 / *d as f64;
                if v >= 1.0 {
                    write!(f, "{}", format_value(v))
                } else {
                    write!(f, "{}/{}", n, d)
                }
            }
            Rational::Float(v) => write!(f, "{}", format_value(*v)),
        }
    }
}
#[derive(Debug, Default, Clone)]
pub struct ExifInfo {
    // 拍摄日期
    pub created_time: Option<ExifDateTime>,
    // 品牌
    pub make: Option<String>,
    // 机型
    pub model: Option<String>,
    // 快门速度, (1/100s)
    pub exposure_time: Option<Rational>,
    // 光圈
    pub f_number: Option<Rational>,
    // 感光度
    pub isospeed_ratings: Option<u32>,
    // 曝光补偿
    pub exposure_bias_value: Option<Rational>,
    // 实际焦距
    pub focal_length: Option<Rational>,
    // 白平衡模式
    pub white_balance_mode: Option<u16>,
    // 等效35mm焦距
    pub focal_length_in35mm_film: Option<u32>,
    // 镜头生产商
    pub lens_make: Option<String>,
    // 镜头型号
    pub lens_model: Option<String>,
}

impl ExifInfo {
    pub async fn new(path: &Path) -> Result<Self> {
        let exif = read_exif_async(path)
            .await
            .with_context(|| format!("Failed to read exif, file: {:?}", path))?;
        let get = |tag| find_value(&exif, tag);
        let to_string = |v: &EntryValue| v.as_str().map(|s| s.to_string());
        Ok(Self {
            created_time: get(ExifTag::CreateDate).and_then(|v| v.as_datetime()),
            make: get(ExifTag::Make).and_then(to_string),
            model: get(ExifTag::Model).and_then(to_string),
            exposure_time: get(ExifTag::ExposureTime).and_then(format_value),
            f_number: get(ExifTag::FNumber).and_then(format_value),
            isospeed_ratings: get(ExifTag::ISOSpeedRatings).and_then(format_iso),
            exposure_bias_value: get(ExifTag::ExposureBiasValue).and_then(format_value),
            focal_length: get(ExifTag::FocalLength).and_then(format_value),
            white_balance_mode: get(ExifTag::WhiteBalanceMode).and_then(|v| v.as_u16()),
            focal_length_in35mm_film: get(ExifTag::FocalLengthIn35mmFilm).and_then(|v| v.as_u32()),
            lens_make: get(ExifTag::LensMake).and_then(to_string),
            lens_model: get(ExifTag::LensModel).and_then(to_string),
        })
    }

    /// 渲染为底部水印文字，缺失的项自动跳过；全部缺失时返回 `None`。
    pub fn to_caption(&self) -> Option<String> {
        // [TODO] 临时
        let parts: Vec<String> = [
            self.isospeed_ratings.map(|v| v.to_string()),
            self.exposure_time.as_ref().map(|v| v.to_string()),
            self.f_number.as_ref().map(|v| v.to_string()),
            self.focal_length_in35mm_film.map(|v| v.to_string()),
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

/// 在所有 IFD 中查找指定 tag 的首个条目（拍摄参数通常位于 Exif 子 IFD 中）。
fn find_value<'a>(exif: &'a Exif, tag: ExifTag) -> Option<&'a EntryValue> {
    exif.iter()
        .find(|e| e.tag.tag() == Some(tag))
        .map(|e| e.value)
}

fn format_iso(value: &EntryValue) -> Option<u32> {
    match value {
        // 部分相机将 ISO 存为数组（如 [100]）
        EntryValue::U16Array(v) => v.first().copied().map(|n| n as u32),
        EntryValue::U32Array(v) => v.first().copied(),
        _ => value.try_as_integer().and_then(|n| u32::try_from(n).ok()),
    }
}

fn format_value(value: &EntryValue) -> Option<Rational> {
    if let Some(r) = value.as_urational() {
        let n = r.numerator();
        let d = r.denominator();
        if d == 0 {
            return None;
        }
        return Some(Rational::Fraction(n, d));
    }
    value.try_as_float().map(|s| Rational::Float(s))
}

#[cfg(test)]
mod tests {

    use crate::photo::Photo;

    #[tokio::test]
    async fn test_dump_exif() {
        let path = "./test_images/DSC_4587.jpg";
        let photo = Photo::new(&path).await.unwrap();
        dbg!(photo.exif);
    }
}
