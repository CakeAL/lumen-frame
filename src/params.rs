use std::path::PathBuf;

use crate::Position;

/// 水印生成参数。
#[derive(Debug, Clone)]
pub struct WatermarkParams {
    /// 输出文件夹
    pub output_folder: Option<PathBuf>,
    /// 边框比例(上下，左右)。例如 `0.10` 表示输出尺寸 = 原尺寸 * `1.10`。
    pub border_ratio: (f64, f64),
    /// 左右边框是否应该和上下边框等宽
    pub border_equal: bool,
    /// 固定宽高比(长，高)
    pub aspect_ratio: Option<(f64, f64)>,
    /// 图片位置，默认为Center
    pub position: Position,
    /// 背景颜色 (R, G, B)，默认纯白。
    pub background: [u8; 3],
    /// 圆角大小，相对于图片高度的比例，默认为0.02，即如果图片高度1000px，那么圆角半径为20px
    pub border_radius: f64,
    /// 阴影大小，相对于图片高度的比例，默认为0.06，即如果图片高度1000px，那么阴影宽度为60px
    pub shadow_size: f64,
    /// 阴影浓度，默认为1.5
    pub shadow_density: f64,
    /// 是否纯色背景
    pub solid_background: bool,
    /// 背景模糊程度, min: 0, max: 1000, default: 15.0
    pub blur_sigma: f64,
    /// 输出 JPEG 质量 1-100，默认 95。
    pub quality: i32,
}

impl Default for WatermarkParams {
    fn default() -> Self {
        let picture_folder = dirs::picture_dir().map(|p| p.join("watermark"));
        Self {
            output_folder: picture_folder,
            border_ratio: (0.10, 0.10),
            border_equal: false,
            aspect_ratio: None,
            background: [255, 255, 255],
            quality: 95,
            border_radius: 0.02,
            shadow_size: 0.06,
            shadow_density: 1.5,
            blur_sigma: 15.0,
            solid_background: false,
            position: Position::Center,
        }
    }
}
