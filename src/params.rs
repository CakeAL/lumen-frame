use std::path::PathBuf;

/// 水印生成参数。
#[derive(Debug, Clone)]
pub struct WatermarkParams {
    /// 源照片路径。
    pub input_path: PathBuf,
    /// 输出图片路径。
    pub output_path: PathBuf,
    /// 边框比例，同时作用于宽高。例如 `0.05` 表示输出尺寸 = 原尺寸 * `1.05`。
    pub border_ratio: f64,
    /// 背景颜色 (R, G, B)，默认纯白。
    pub background: [u8; 3],
    /// 信息文字颜色 (R, G, B)，默认深灰。
    pub text_color: [u8; 3],
    /// 信息文字字体（Pango 描述，例如 `"sans 48"`）；`None` 时按边框高度自动估算。
    pub font: Option<String>,
    /// 文字渲染 DPI，默认 72（此时 Pango 字号 1pt ≈ 1px）。
    pub dpi: i32,
    /// 输出 JPEG 质量 1-100，默认 95。
    pub quality: i32,
}

impl WatermarkParams {
    /// 使用默认参数创建实例：边框 5%、纯白背景、深灰文字。
    pub fn new(input_path: impl Into<PathBuf>, output_path: impl Into<PathBuf>) -> Self {
        Self {
            input_path: input_path.into(),
            output_path: output_path.into(),
            border_ratio: 0.05,
            background: [255, 255, 255],
            text_color: [60, 60, 60],
            font: None,
            dpi: 72,
            quality: 95,
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