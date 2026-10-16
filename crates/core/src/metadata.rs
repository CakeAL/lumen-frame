//! EXIF 文本输入的共享解析规则。
use crate::{
    features::geolocation::Location,
    media::{ExifInfo, Rational},
};
use chrono::NaiveDateTime;
use nom_exif::{ExifDateTime, GPSInfo};
use serde::{Deserialize, Serialize};
const DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub fn format_created_time(value: Option<ExifDateTime>) -> String {
    value
        .map(|value| value.into_naive().format(DATE_FORMAT).to_string())
        .unwrap_or_default()
}

pub fn parse_created_time(value: &str) -> Result<Option<ExifDateTime>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    for format in [DATE_FORMAT, "%Y:%m:%d %H:%M:%S"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(Some(ExifDateTime::Naive(value)));
        }
    }
    Err("拍摄日期格式无效，请使用 YYYY-MM-DD HH:MM:SS".to_owned())
}

pub fn format_rational(value: Option<&Rational>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

pub fn parse_rational(value: &str, label: &str) -> Result<Option<Rational>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator
            .trim()
            .parse::<i32>()
            .map_err(|_| format!("{label}格式无效，请输入小数或分数"))?;
        let denominator = denominator
            .trim()
            .parse::<i32>()
            .map_err(|_| format!("{label}格式无效，请输入小数或分数"))?;
        if denominator == 0 {
            return Err(format!("{label}的分母不能为 0"));
        }
        return Ok(Some(Rational::Fraction(numerator, denominator)));
    }
    let value = value
        .parse::<f64>()
        .map_err(|_| format!("{label}格式无效，请输入小数或分数"))?;
    if !value.is_finite() {
        return Err(format!("{label}必须是有限数值"));
    }
    Ok(Some(Rational::Float(value)))
}

pub fn parse_integer<T>(value: &str, label: &str) -> Result<Option<T>, String>
where
    T: std::str::FromStr,
{
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<T>()
        .map(Some)
        .map_err(|_| format!("{label}必须是非负整数"))
}

pub fn parse_gps(value: &str) -> Result<Option<GPSInfo>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let gps = value
        .parse::<GPSInfo>()
        .map_err(|_| "GPS 格式无效，请使用 ISO 6709，例如 +22.54290+114.05960/".to_owned())?;
    Location::from_gps(&gps)
        .ok_or_else(|| "GPS 坐标超出范围：纬度 -90 至 90，经度 -180 至 180".to_owned())?;
    Ok(Some(gps))
}

/// JSON 边界保留可读的分数、日期和 ISO 6709，不泄露解析库类型。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExifData {
    pub created_time: String,
    pub make: String,
    pub model: String,
    pub exposure_time: String,
    pub f_number: String,
    pub iso: Option<u32>,
    pub exposure_bias: String,
    pub focal_length: String,
    pub white_balance: Option<u16>,
    pub focal_length_35mm: Option<u16>,
    pub lens_make: String,
    pub lens_model: String,
    pub gps: String,
}
impl From<&ExifInfo> for ExifData {
    fn from(e: &ExifInfo) -> Self {
        Self {
            created_time: format_created_time(e.created_time),
            make: e.make.clone().unwrap_or_default(),
            model: e.model.clone().unwrap_or_default(),
            exposure_time: format_rational(e.exposure_time.as_ref()),
            f_number: format_rational(e.f_number.as_ref()),
            iso: e.isospeed_ratings,
            exposure_bias: format_rational(e.exposure_bias_value.as_ref()),
            focal_length: format_rational(e.focal_length.as_ref()),
            white_balance: e.white_balance_mode,
            focal_length_35mm: e.focal_length_in35mm_film,
            lens_make: e.lens_make.clone().unwrap_or_default(),
            lens_model: e.lens_model.clone().unwrap_or_default(),
            gps: e
                .gps_info
                .as_ref()
                .map(GPSInfo::to_iso6709)
                .unwrap_or_default(),
        }
    }
}
impl ExifData {
    pub fn apply_patch(
        original: &ExifInfo,
        changes: serde_json::Map<String, serde_json::Value>,
    ) -> anyhow::Result<ExifInfo> {
        let mut value = serde_json::to_value(Self::from(original))?;
        for (key, patch) in &changes {
            value
                .as_object_mut()
                .unwrap()
                .insert(key.clone(), patch.clone());
        }
        let data: Self = serde_json::from_value(value)?;
        let parse = || -> Result<ExifInfo, String> {
            Ok(ExifInfo {
                created_time: if changes.contains_key("created_time") {
                    parse_created_time(&data.created_time)?
                } else {
                    original.created_time
                },
                make: optional_text(&data.make),
                model: optional_text(&data.model),
                exposure_time: parse_rational(&data.exposure_time, "快门")?,
                f_number: parse_rational(&data.f_number, "光圈")?,
                isospeed_ratings: data.iso,
                exposure_bias_value: parse_rational(&data.exposure_bias, "曝光补偿")?,
                focal_length: parse_rational(&data.focal_length, "焦距")?,
                white_balance_mode: data.white_balance,
                focal_length_in35mm_film: data.focal_length_35mm,
                lens_make: optional_text(&data.lens_make),
                lens_model: optional_text(&data.lens_model),
                gps_info: if changes.contains_key("gps") {
                    parse_gps(&data.gps)?
                } else {
                    original.gps_info.clone()
                },
            })
        };
        parse().map_err(anyhow::Error::msg)
    }
}
