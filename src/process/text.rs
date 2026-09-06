use libvips::{VipsImage, ops};
use nom_exif::ExifDateTime;
use regex::Regex;

use crate::{helper::auto_color, params::WatermarkParams};

use crate::{
    Position,
    photo::{ExifInfo, Rational},
};

/// 需要渲染的多行文本
#[derive(Debug, Clone)]
pub struct Text {
    /// 每行文本模板
    pub template: Vec<String>,
    /// 每行文本参数
    pub text_params: Vec<TextParams>,
    /// 文本位置
    pub position: Position,
}

impl Text {
    /// 计算多行文本占用高度px，用于计算有字体一侧的margin宽度
    /// 返回 (文本相对于图片的位置, 文本高度)
    pub fn cal_height(&self, img_h: i32) -> (Position, i32) {
        if self.text_params.is_empty() {
            return (Position::Bottom, 0);
        }
        let img_h = img_h as f64;
        let text_height: i32 = self
            .text_params
            .iter()
            .map(|text_params| (text_params.size * img_h).round() as i32)
            .sum();
        let spacing: i32 = self
            .text_params
            .windows(2)
            .map(|lines| (lines[0].size * img_h * lines[0].line_spacing).round() as i32)
            .sum();
        (self.position, text_height + spacing)
    }
}

#[derive(Debug, Clone, Default)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Default)]
pub enum TextDirection {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub struct TextParams {
    pub font: String,
    /// 该尺寸系与图片背景高度的百分比，默认为0.03：如果图片高度1000px，那么字体高度为30px
    pub size: f64,
    /// 行间距，默认为0.3
    pub line_spacing: f64,
    /// None的时候即自动颜色
    pub color: Option<[u8; 3]>,
    pub italic: bool,
    pub bold: bool,
    pub align: TextAlign,
    /// 文字方向
    pub direction: TextDirection,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            font: "Arial".to_string(),
            size: 0.03,
            line_spacing: 0.3,
            color: None,
            italic: false,
            bold: false,
            align: TextAlign::default(),
            direction: TextDirection::default(),
        }
    }
}

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

fn find_make_logo(make: &str, watermark_params: &WatermarkParams) -> Option<VipsImage> {
    let make = make.replace("CORPORATION", "").trim().to_lowercase();
    if auto_color(watermark_params) {
        ops::svgload(&format!("./static/logo/{}-b.svg", make)).ok()
    } else {
        ops::svgload(&format!("./static/logo/{}-w.svg", make)).ok()
    }
}
