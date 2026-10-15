//! 水印合成的可序列化输入模型。

use std::path::PathBuf;
use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::rotation::Rotation;

mod gradient;
pub use gradient::{BackgroundGradient, GradientPalette, PaletteTheme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundKind {
    Photo,
    Solid,
    Gradient,
}

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
    /// 本机默认字体，由应用设置提供，不写入预设。
    #[serde(skip, default = "default_font")]
    pub default_font: String,
    /// 当前照片的自定义文本，随照片快照保存，不属于可复用预设。
    #[serde(skip)]
    pub custom_text: String,
    /// 本机素材库的不可变快照，不写入预设；导出期间保留素材字节。
    #[serde(skip)]
    pub custom_logos: BTreeMap<String, Arc<Vec<u8>>>,
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
    /// 单独保留旧纯色开关，旧预设无需迁移；渐变启用时优先使用渐变。
    pub gradient_background: bool,
    pub gradient: BackgroundGradient,
    /// 色卡来源保留代表色序号，应用到其它照片时由那张照片重新求值。
    pub gradient_palette: GradientPalette,
    pub blur_sigma: f64,
    /// 全部照片共用的本机 JPEG 导出质量，不写入预设。
    #[serde(skip, default = "default_jpeg_quality")]
    pub quality: i32,
    pub rotation: Rotation,
}

impl Default for WatermarkParams {
    fn default() -> Self {
        Self {
            output_folder: dirs::picture_dir().map(|path| path.join("watermark")),
            default_font: default_font(),
            custom_text: String::new(),
            custom_logos: BTreeMap::new(),
            border_ratio: (0.05, 0.05, 0.05, 0.05),
            border_equal: false,
            aspect_ratio: None,
            position: Placement::Center,
            background: [255, 255, 255],
            border_radius: 0.02,
            shadow_size: 0.06,
            shadow_density: 1.2,
            solid_background: false,
            gradient_background: false,
            gradient: BackgroundGradient::default(),
            gradient_palette: GradientPalette::default(),
            blur_sigma: 15.0,
            quality: DEFAULT_JPEG_QUALITY,
            rotation: Rotation::None,
        }
    }
}

impl WatermarkParams {
    pub fn background_kind(&self) -> BackgroundKind {
        if self.gradient_background {
            BackgroundKind::Gradient
        } else if self.solid_background {
            BackgroundKind::Solid
        } else {
            BackgroundKind::Photo
        }
    }

    pub fn set_background_kind(&mut self, kind: BackgroundKind) {
        self.gradient_background = kind == BackgroundKind::Gradient;
        self.solid_background = kind == BackgroundKind::Solid;
    }

    /// 自动文字颜色以渐变中点作参考，用户仍可为文字组指定固定颜色。
    pub(crate) fn text_background_color(&self) -> Option<[u8; 3]> {
        match self.background_kind() {
            BackgroundKind::Gradient => Some(self.gradient.color_at(0.5)),
            BackgroundKind::Solid => Some(self.background),
            BackgroundKind::Photo => None,
        }
    }
}

pub const DEFAULT_TIME_FORMAT: &str = "%Y/%m/%d";
pub const DEFAULT_FONT: &str = "Arial";
pub const DEFAULT_JPEG_QUALITY: i32 = 95;

fn default_jpeg_quality() -> i32 {
    DEFAULT_JPEG_QUALITY
}

fn default_font() -> String {
    DEFAULT_FONT.to_owned()
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<GroupAttachment>,
}

/// 指向同一不可变预设快照中的文字组；UI 编辑期间使用稳定组 ID，保存时投影成序号。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupAttachment {
    pub target: usize,
    pub side: Placement,
    /// 与目标组的距离，占照片高度的比例。
    pub gap: f64,
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
    /// 空字符串表示继承应用的默认字体。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub font: String,
    pub size: f64,
    /// 自定义 Logo 可见内容的高度占照片高度的比例；旧预设未设置时沿用字号比例。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_size: Option<f64>,
    pub line_spacing: f64,
    pub color: Option<[u8; 3]>,
    pub italic: bool,
    pub bold: bool,
    pub align: TextAlign,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            font: String::new(),
            size: 0.03,
            logo_size: None,
            line_spacing: 1.3,
            color: None,
            italic: false,
            bold: false,
            align: TextAlign::default(),
        }
    }
}

impl TextParams {
    pub(crate) fn resolved_font<'a>(&'a self, default_font: &'a str) -> &'a str {
        if self.font.trim().is_empty() {
            if default_font.trim().is_empty() {
                DEFAULT_FONT
            } else {
                default_font
            }
        } else {
            &self.font
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_FONT, TextParams};

    #[test]
    fn text_font_uses_global_default_only_when_unspecified() {
        let inherited = TextParams::default();
        assert_eq!(inherited.resolved_font("Menlo"), "Menlo");
        assert_eq!(inherited.resolved_font(""), DEFAULT_FONT);

        let explicit = TextParams {
            font: "Times New Roman".to_owned(),
            ..Default::default()
        };
        assert_eq!(explicit.resolved_font("Menlo"), "Times New Roman");
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
            attachment: None,
        }
    }
}
