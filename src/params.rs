use std::path::PathBuf;

/// 生成纯白底画布所需的参数。
#[derive(Debug, Clone)]
pub struct CanvasParams {
    /// 源照片路径，用于读取原始长宽。
    pub input_path: PathBuf,
    /// 新生成图片的输出路径。
    pub output_path: PathBuf,
    /// 宽度额外比例。例如 0.05 表示新宽度 = 原宽度 * 1.05。
    pub width_pad_ratio: f64,
    /// 背景颜色 (R, G, B)，默认纯白。
    pub background: [u8; 3],
}

impl CanvasParams {
    /// 使用默认参数创建实例：宽度增加 5%，纯白背景。
    pub fn new(input_path: impl Into<PathBuf>, output_path: impl Into<PathBuf>) -> Self {
        Self {
            input_path: input_path.into(),
            output_path: output_path.into(),
            width_pad_ratio: 0.05,
            background: [255, 255, 255],
        }
    }
}
