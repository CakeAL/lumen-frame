//! 水印合成的可序列化输入模型。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 元素相对于照片内容的放置边。
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    Center,
    Up,
    Right,
    Bottom,
    Left,
}

/// 水印生成参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatermarkParams {
    /// 输出文件夹。这是本机环境状态，不写入预设。
    #[serde(skip)]
    pub output_folder: Option<PathBuf>,
    /// 边框比例（上、下、左、右）。
    pub border_ratio: (f64, f64, f64, f64),
    pub border_equal: bool,
    pub aspect_ratio: Option<(f64, f64)>,
    pub position: Placement,
    pub background: [u8; 3],
    pub border_radius: f64,
    pub shadow_size: f64,
    pub shadow_density: f64,
    pub solid_background: bool,
    pub blur_sigma: f64,
    pub quality: i32,
}

impl Default for WatermarkParams {
    fn default() -> Self {
        Self {
            output_folder: dirs::picture_dir().map(|path| path.join("watermark")),
            border_ratio: (0.05, 0.05, 0.05, 0.05),
            border_equal: false,
            aspect_ratio: None,
            position: Placement::Center,
            background: [255, 255, 255],
            border_radius: 0.02,
            shadow_size: 0.06,
            shadow_density: 1.2,
            solid_background: false,
            blur_sigma: 15.0,
            quality: 95,
        }
    }
}

pub const DEFAULT_TIME_FORMAT: &str = "%Y/%m/%d";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextGroup {
    pub text: Text,
    pub position: Placement,
    pub direction: TextDirection,
    pub align: TextAlign,
    /// 文字与相邻图片边缘的留白比例；上下位置沿水平方向，左右位置沿垂直方向。
    #[serde(default)]
    pub padding: f64,
    pub time_format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub template: Vec<String>,
    pub text_params: Vec<TextParams>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextDirection {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextParams {
    pub font: String,
    pub size: f64,
    pub line_spacing: f64,
    pub color: Option<[u8; 3]>,
    pub italic: bool,
    pub bold: bool,
    pub align: TextAlign,
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
        }
    }
}

impl Default for Text {
    fn default() -> Self {
        Self {
            template: vec![
                "{Logo} {型号}".to_owned(),
                "{拍摄日期} {等效焦距}mm f/{光圈} {快门}s ISO{ISO}".to_owned(),
            ],
            text_params: vec![
                TextParams {
                    size: 0.03,
                    bold: true,
                    ..Default::default()
                },
                TextParams {
                    size: 0.022,
                    ..Default::default()
                },
            ],
        }
    }
}

impl Default for TextGroup {
    fn default() -> Self {
        Self {
            text: Text::default(),
            position: Placement::Bottom,
            direction: TextDirection::Horizontal,
            align: TextAlign::Center,
            padding: 0.0,
            time_format: DEFAULT_TIME_FORMAT.to_owned(),
        }
    }
}
