//! 照片 EXIF 元数据读取与领域化格式。

use std::{fmt::Display, path::Path};

use anyhow::{Context as _, Result};
use nom_exif::{
    EntryValue, Exif, ExifDateTime, ExifTag, GPSInfo, MediaParser, MediaSource, read_exif_async,
};
use num_integer::Integer;

/// 光圈、快门等既可能是分数，也可能是浮点数。
#[derive(Debug, Clone)]
pub enum Rational {
    Fraction(i32, i32),
    Float(f64),
}

impl Display for Rational {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format_value = |value: f64| {
            format!("{value:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        match self {
            Rational::Fraction(numerator, denominator) => {
                let gcd = numerator.gcd(denominator);
                let numerator = numerator / gcd;
                let denominator = denominator / gcd;
                if denominator == 1 {
                    write!(formatter, "{numerator}")
                } else {
                    let value = numerator as f64 / denominator as f64;
                    if value >= 1.0 {
                        write!(formatter, "{}", format_value(value))
                    } else {
                        write!(formatter, "{numerator}/{denominator}")
                    }
                }
            }
            Rational::Float(value) => {
                // 只有正的小数才可能表示以秒计的快门速度。负数通常是曝光补偿；把它
                // 取倒数再转成 u32 会饱和成 4294967295，最终显示为无效的
                // `1/4294967295`。
                if value.is_finite() && *value > 0.0 && *value < 1.0 {
                    write!(formatter, "1/{}", (1.0 / value).round() as u32)
                } else {
                    write!(formatter, "{}", format_value(*value))
                }
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExifInfo {
    pub created_time: Option<ExifDateTime>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub exposure_time: Option<Rational>,
    pub f_number: Option<Rational>,
    pub isospeed_ratings: Option<u32>,
    pub exposure_bias_value: Option<Rational>,
    pub focal_length: Option<Rational>,
    pub white_balance_mode: Option<u16>,
    pub focal_length_in35mm_film: Option<u16>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub gps_info: Option<GPSInfo>,
}

impl ExifInfo {
    pub async fn new(path: &Path) -> Result<Self> {
        let exif = match read_exif_async(path).await {
            Ok(exif) => exif,
            Err(original_error) => {
                let bytes = tokio::fs::read(path)
                    .await
                    .with_context(|| format!("Failed to read exif, file: {path:?}"))?;
                read_exif_from_jpeg_app1(&bytes).with_context(|| {
                    format!("Failed to read exif, file: {path:?}; original error: {original_error}")
                })?
            }
        };
        Ok(Self::from_exif(&exif))
    }

    /// 阻塞读取 EXIF，供没有 Tokio 运行时的后台工作线程使用。
    pub fn read(path: &Path) -> Result<Self> {
        let exif = match nom_exif::read_exif(path) {
            Ok(exif) => exif,
            Err(original_error) => {
                let bytes = std::fs::read(path)
                    .with_context(|| format!("Failed to read exif, file: {path:?}"))?;
                read_exif_from_jpeg_app1(&bytes).with_context(|| {
                    format!("Failed to read exif, file: {path:?}; original error: {original_error}")
                })?
            }
        };
        Ok(Self::from_exif(&exif))
    }

    fn from_exif(exif: &Exif) -> Self {
        let get = |tag| find_value(exif, tag);
        let to_string = |value: &EntryValue| value.as_str().map(str::to_owned);
        Self {
            created_time: get(ExifTag::CreateDate).and_then(|value| value.as_datetime()),
            make: get(ExifTag::Make).and_then(to_string),
            model: get(ExifTag::Model).and_then(to_string),
            exposure_time: get(ExifTag::ExposureTime).and_then(format_value),
            f_number: get(ExifTag::FNumber).and_then(format_value),
            isospeed_ratings: get(ExifTag::ISOSpeedRatings).and_then(format_iso),
            exposure_bias_value: get(ExifTag::ExposureBiasValue).and_then(format_value),
            focal_length: get(ExifTag::FocalLength).and_then(format_value),
            white_balance_mode: get(ExifTag::WhiteBalanceMode).and_then(|value| value.as_u16()),
            focal_length_in35mm_film: get(ExifTag::FocalLengthIn35mmFilm)
                .and_then(|value| value.as_u16()),
            lens_make: get(ExifTag::LensMake).and_then(to_string),
            lens_model: get(ExifTag::LensModel).and_then(to_string),
            gps_info: exif.gps_info().cloned(),
        }
    }
}

/// 兼容 XMP APP1 位于 EXIF APP1 之前的 JPEG。
fn read_exif_from_jpeg_app1(jpeg: &[u8]) -> Result<Exif> {
    anyhow::ensure!(jpeg.starts_with(&[0xff, 0xd8]), "不是 JPEG 文件");
    let mut offset = 2;
    while offset + 4 <= jpeg.len() {
        anyhow::ensure!(jpeg[offset] == 0xff, "JPEG 标记损坏");
        let marker = jpeg[offset + 1];
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let length = u16::from_be_bytes([jpeg[offset + 2], jpeg[offset + 3]]) as usize;
        anyhow::ensure!(
            length >= 2 && offset + 2 + length <= jpeg.len(),
            "JPEG 段长度损坏"
        );
        let payload = offset + 4;
        if marker == 0xe1 && jpeg.get(payload..payload + 6) == Some(b"Exif\0\0") {
            let end = offset + 2 + length;
            let mut minimal_jpeg = Vec::with_capacity(end - offset + 2);
            minimal_jpeg.extend_from_slice(&[0xff, 0xd8]);
            minimal_jpeg.extend_from_slice(&jpeg[offset..end]);
            let source = MediaSource::from_memory(minimal_jpeg)?;
            let mut parser = MediaParser::new();
            return parser
                .parse_exif(source)
                .map(Exif::from)
                .map_err(Into::into);
        }
        offset += 2 + length;
    }
    anyhow::bail!("JPEG 中未找到 EXIF APP1 段")
}

fn find_value(exif: &Exif, tag: ExifTag) -> Option<&EntryValue> {
    exif.entries()
        .find(|entry| entry.tag().tag() == Some(tag))
        .map(|entry| entry.value())
}

fn format_iso(value: &EntryValue) -> Option<u32> {
    match value {
        EntryValue::U16Array(values) => values.first().copied().map(u32::from),
        EntryValue::U32Array(values) => values.first().copied(),
        _ => value
            .try_as_integer()
            .and_then(|number| u32::try_from(number).ok()),
    }
}

fn format_value(value: &EntryValue) -> Option<Rational> {
    if let Some(rational) = value.as_irational() {
        let numerator = rational.numerator();
        let denominator = rational.denominator();
        if denominator == 0 {
            return None;
        }
        return Some(Rational::Fraction(numerator, denominator));
    }
    value.try_as_float().map(Rational::Float)
}

#[cfg(test)]
mod tests {
    use super::Rational;

    #[test]
    fn float_display_only_uses_reciprocal_for_positive_shutter_values() {
        assert_eq!(Rational::Float(1.0 / 400.0).to_string(), "1/400");
        assert_eq!(Rational::Float(-1.0 / 3.0).to_string(), "-0.33");
        assert_eq!(Rational::Float(0.0).to_string(), "0");
    }
}
