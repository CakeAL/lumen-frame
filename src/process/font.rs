use nom_exif::ExifDateTime;
use regex::Regex;

use crate::photo::{ExifInfo, Rational};

/// 根据给定的模板生成文字
pub fn render_exif_template(template: &str, exif: &ExifInfo, time_format: &str) -> String {
    let re = Regex::new(r"\{([^{}]+)\}").unwrap();

    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];

        resolve_exif_key_name(key, exif, time_format).unwrap_or_else(|| caps[0].to_string())
    })
    .into_owned()
}

fn resolve_exif_key_name(key: &str, exif: &ExifInfo, time_format: &str) -> Option<String> {
    match key {
        "拍摄日期" => exif
            .created_time
            .as_ref()
            .map(|time| format_created_time(time, time_format)),
        "品牌" => exif.make.clone(),
        "机型" => exif.model.clone(),
        "快门" => exif.exposure_time.as_ref().map(ToString::to_string),
        "光圈" => exif.f_number.as_ref().map(format_fnumber),
        "ISO" => exif.isospeed_ratings.map(|v| v.to_string()),
        "曝光补偿" => exif.exposure_bias_value.as_ref().map(ToString::to_string),
        "实际焦距" => exif.focal_length.as_ref().map(ToString::to_string),
        "白平衡" => exif.white_balance_mode.map(|v| v.to_string()),
        "等效焦距" => exif.focal_length_in35mm_film.map(|v| v.to_string()),
        "镜头生产商" => exif.lens_make.clone(),
        "镜头型号" => exif.lens_model.clone(),
        _ => None,
    }
}

fn format_created_time(value: &ExifDateTime, time_format: &str) -> String {
    if let Some(dt) = value.aware() {
        dt.format(time_format).to_string()
    } else {
        value.into_naive().format(time_format).to_string()
    }
}

fn format_fnumber(value: &Rational) -> String {
    let format_value = |v: f64| {
        format!("{v:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    };
    match value {
        Rational::Float(v) => format_value(*v),
        Rational::Fraction(n, d) => {
            let v = *n as f64 / *d as f64;
            format_value(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use nom_exif::ExifDateTime;

    use crate::{
        photo::{ExifInfo, Rational},
        process::font::render_exif_template,
    };

    #[test]
    fn test_render_exif_template() {
        let template = "{拍摄日期} {等效焦距}mm f/{光圈} {快门}s ISO{ISO} {曝光补偿}EV";
        let exif_info = ExifInfo {
            created_time: Some(ExifDateTime::Aware(Local::now().into())),
            focal_length_in35mm_film: Some(50),
            exposure_time: Some(Rational::Fraction(1, 250)),
            f_number: Some(Rational::Fraction(18, 10)),
            exposure_bias_value: Some(Rational::Fraction(-1, 3)),
            focal_length: Some(Rational::Fraction(500, 10)),
            ..Default::default()
        };
        let text = render_exif_template(template, &exif_info, "%Y/%m/%d %H:%M:%S");
        dbg!(text);
    }
}
