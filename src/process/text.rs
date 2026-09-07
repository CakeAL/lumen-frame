use libvips::{Result, VipsImage, ops};
use nom_exif::ExifDateTime;
use regex::Regex;

use crate::{
    helper::{auto_color, escape_xml},
    params::WatermarkParams,
};

use crate::{
    Position,
    photo::{ExifInfo, Rational},
};

pub type SvgString = String;

/// 需要渲染的多行文本
#[derive(Debug, Clone)]
pub struct Text {
    /// 每行文本模板
    pub template: Vec<String>,
    /// 每行文本参数
    pub text_params: Vec<TextParams>,
    /// 文本位置
    pub position: Position,
    /// 时间格式
    pub time_format: String,
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
            .map(|lines| (lines[0].size * img_h * (lines[0].line_spacing - 1.0)).round() as i32)
            .sum();
        (self.position, text_height + spacing)
    }

    pub fn render_text(
        &self,
        exif: &ExifInfo,
        img_h: i32,
        canvas_w: i32,
        watermark_params: &WatermarkParams,
    ) -> Result<Option<SvgString>> {
        // 行数
        let line_count = self.template.len().min(self.text_params.len());
        if line_count == 0 {
            return Ok(None);
        }

        // 根据模板生成文字
        let texts: Vec<String> = self
            .template
            .iter()
            .map(|t| render_exif_template(t, exif, &self.time_format))
            .collect();

        // 计算每一行的字号
        let font_sizes: Vec<f32> = self
            .text_params
            .iter()
            .map(|params| (params.size * img_h as f64) as f32)
            .collect();

        // 计算svg高度和宽度
        let svg_height: f32 = self
            .text_params
            .iter()
            .take(line_count)
            .zip(font_sizes.iter())
            .map(|(params, font_sizes)| font_sizes * params.line_spacing as f32)
            .sum();
        let svg_width = canvas_w as f32;

        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"
            width="{svg_width}"
            height="{svg_height}"
            viewBox="0 0 {svg_width} {svg_height}">"#
        );
        let mut y = 0.0;

        for ((text, params), font_size) in texts
            .iter()
            .zip(self.text_params.iter().take(line_count))
            .zip(font_sizes.iter())
        {
            // SVG text 的 y 是 baseline
            let baseline = y + *font_size;

            let color = if let Some(rgb) = params.color {
                format!("rgb({},{},{})", rgb[0], rgb[1], rgb[2])
            } else {
                if auto_color(watermark_params) {
                    "black".into()
                } else {
                    "white".into()
                }
            };
            let font_style = if params.italic { "italic" } else { "normal" };

            let font_weight = if params.bold { "bold" } else { "normal" };

            let text_anchor = match params.align {
                TextAlign::Left => "start",
                TextAlign::Center => "middle",
                TextAlign::Right => "end",
            };

            let x = match params.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => svg_width / 2.0,
                TextAlign::Right => svg_width,
            };
            svg.push_str(&format!(
                r#"<text
                    x="{x}"
                    y="{baseline}"
                    font-family="{font_family}"
                    font-size="{font_size}px"
                    font-style="{font_style}"
                    font-weight="{font_weight}"
                    text-anchor="{text_anchor}"
                    fill="{color}">
                    {text}
                </text>"#,
                font_family = params.font,
                text = escape_xml(text),
            ));
            y += font_size * params.line_spacing as f32;
        }
        svg.push_str("</svg>");

        Ok(Some(svg))
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
    /// 行间距，默认为1.3，即字体高度为30px，那么行间距是9px
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
            line_spacing: 1.3,
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
        "Logo" => exif.make.clone(),
        "型号" => exif
            .model
            .as_ref()
            .map(|m| format_model(m, &exif.make.clone().unwrap_or_default())),
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

fn format_model(model: &str, make: &str) -> String {
    let make = make.replace("CORPORATION", "").trim().to_lowercase();
    if make.contains("sony") {
        model.replace("ILCE-", "α").to_lowercase()
    } else if make.contains("nikon") {
        model
            .replace(&make.to_uppercase(), "")
            .trim()
            .replace('Z', "ℤ")
            .replace('z', "ℤ")
            .split('_')
            .collect::<Vec<_>>()
            .split_last()
            .map(|(last, rest)| {
                if let Ok(num) = last.parse::<i32>() {
                    format!("{} {}", rest.join(" "), crate::helper::to_roman(num))
                } else {
                    format!("{} {}", rest.join(" "), last)
                }
            })
            .unwrap_or_else(|| model.trim().to_string())
    } else {
        model.to_owned()
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
