use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Position;

/// 水印生成参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatermarkParams {
    /// 输出文件夹。
    ///
    /// 这一项不写进预设：它属于「运行这台机器时的环境」，跟着预设走只会让预设换台机器
    /// 之后导出到不存在的位置。
    ///
    /// 注意 `skip` 的实际语义：字段不会被写出，但反序列化时整个结构体会先取
    /// `WatermarkParams::default()`，于是这里拿回来的是默认的图片目录，**不是** `None`。
    /// 载入预设的人必须显式保留当前值（`AppView::apply_preset` 就是这么做的）。
    #[serde(skip)]
    pub output_folder: Option<PathBuf>,
    /// 边框比例(上，下，左，右)。例如 `0.05` 表示边框尺寸 = 原尺寸 * `0.05`。
    pub border_ratio: (f64, f64, f64, f64),
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
    /// 阴影浓度，默认为1.2
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
            border_ratio: (0.05, 0.05, 0.05, 0.05),
            border_equal: false,
            aspect_ratio: None,
            background: [255, 255, 255],
            quality: 95,
            border_radius: 0.02,
            shadow_size: 0.06,
            shadow_density: 1.2,
            blur_sigma: 15.0,
            solid_background: false,
            position: Position::Center,
        }
    }
}
