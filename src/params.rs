use std::path::PathBuf;

/// 水印生成参数。
#[derive(Debug, Clone)]
pub struct WatermarkParams {
    /// 源照片路径。
    pub input_path: PathBuf,
    /// 输出图片路径。
    pub output_path: PathBuf,
    /// 边框比例(上下，左右)。例如 `0.10` 表示输出尺寸 = 原尺寸 * `1.10`。
    pub border_ratio: (f32, f32),
    /// 固定宽高比(长，宽)
    pub aspect_ratio: Option<(u8, u8)>,
    /// 背景颜色 (R, G, B)，默认纯白。
    pub background: [u8; 3],
    /// 圆角大小，相对于图片高度的比例，默认为0.02，即如果图片高度1000px，那么圆角半径为20px
    pub border_radius: f32,
    /// 阴影大小，相对于图片高度的比例，默认为0.06，即如果图片高度1000px，那么阴影宽度为60px
    pub shadow_size: f32,
    /// 背景模糊程度，默认为0.15
    pub blur_sigma: f32,
    /// 字体参数
    pub text_params: TextParams,
    /// 输出 JPEG 质量 1-100，默认 95。
    pub quality: i32,
}

impl WatermarkParams {
    pub fn new(input_path: impl Into<PathBuf>, output_path: impl Into<PathBuf>) -> Self {
        Self {
            input_path: input_path.into(),
            output_path: output_path.into(),
            border_ratio: (0.10, 0.10),
            aspect_ratio: None,
            background: [255, 255, 255],
            text_params: TextParams::default(),
            quality: 95,
            border_radius: 0.02,
            shadow_size: 0.06,
            blur_sigma: 0.15,
        }
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
    pub size: f32,
    /// 行间距，默认为1.3
    pub line_spacing: f32,
    pub color: [u8; 3],
    pub italic: bool,
    pub bold: bool,
    pub align: TextAlign,
    /// 相对于图片的位置(上下左右：0123)
    pub position: u8,
    /// 文字方向
    pub direction: TextDirection,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            font: "Arial".to_string(),
            size: 0.03,
            line_spacing: 1.3,
            color: [0, 0, 0],
            italic: false,
            bold: false,
            align: TextAlign::default(),
            position: 1,
            direction: TextDirection::default(),
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
