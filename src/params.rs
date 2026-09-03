use std::path::PathBuf;

/// 水印生成参数。
#[derive(Debug, Clone)]
pub struct WatermarkParams {
    /// 输出文件夹
    pub output_folder: PathBuf,
    /// 边框比例(上下，左右)。例如 `0.10` 表示输出尺寸 = 原尺寸 * `1.10`。
    pub border_ratio: (f64, f64),
    /// 左右边框是否应该和上下边框等宽
    pub border_equal: bool,
    /// 固定宽高比(长，宽)
    pub aspect_ratio: Option<(u8, u8)>,
    /// 背景颜色 (R, G, B)，默认纯白。
    pub background: [u8; 3],
    /// 圆角大小，相对于图片高度的比例，默认为0.02，即如果图片高度1000px，那么圆角半径为20px
    pub border_radius: f64,
    /// 阴影大小，相对于图片高度的比例，默认为0.06，即如果图片高度1000px，那么阴影宽度为60px
    pub shadow_size: f64,
    /// 阴影透明度，默认为0.5，即50%透明度
    pub shadow_opacity: f64,
    /// 是否纯色背景
    pub solid_background: bool,
    /// 背景模糊程度, min: 0, max: 1000, default: 1.5
    pub blur_sigma: f64,
    /// 字体参数
    pub text_params: TextParams,
    /// 输出 JPEG 质量 1-100，默认 95。
    pub quality: i32,
}

impl WatermarkParams {
    pub fn new(output_folder: impl Into<PathBuf>) -> Self {
        Self {
            output_folder: output_folder.into(),
            border_ratio: (0.10, 0.10),
            border_equal: false,
            aspect_ratio: None,
            background: [255, 255, 255],
            text_params: TextParams::default(),
            quality: 95,
            border_radius: 0.02,
            shadow_size: 0.06,
            shadow_opacity: 0.5,
            blur_sigma: 1.5,
            solid_background: false,
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
    pub size: f64,
    /// 行间距，默认为1.3
    pub line_spacing: f64,
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
