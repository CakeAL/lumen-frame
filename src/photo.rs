use nom_exif::{EntryValue, Exif, ExifTag, MediaKind, MediaParser, MediaSource};
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Photo{
    pub path: PathBuf,
    pub is_motion_photo: bool,
    pub is_ultra_hdr_photo: bool,
    pub exif: Option<ExifInfo>,
}

impl Photo {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            is_motion_photo: false,
            is_ultra_hdr_photo: false,
            exif: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExifInfo {
    // 创作者
    pub producer: Option<String>,
    // 拍摄日期
    pub recorded_time: Option<String>,
    // 机型
    pub writing_hardware: Option<String>,
    // 快门速度
    pub shutter_speed_time: Option<String>,
    // 光圈
    pub f_number: Option<String>,
    // 自动曝光模式
    pub auto_exposure_mode: Option<String>,
    // 感光度
    pub iso_sensitivity: Option<String>,
    // 曝光补偿
    pub exposure_bias_value: Option<String>,
    // 是否闪光
    pub flash: Option<String>,
    // 实际焦距
    pub lens_zoom_actual_focal_length: Option<String>,
    // 自动白平衡模式
    pub auto_white_balance_mode: Option<String>,
    // 等效35mm焦距
    pub lens_zoom_35mm_still_camera_equivalent: Option<String>,
    // 镜头生产商
    pub lens_make: Option<String>,
    // 镜头型号
    pub lens_model: Option<String>,
}

impl ExifInfo {
    /// 渲染为底部水印文字，缺失的项自动跳过；全部缺失时返回 `None`。
    pub fn to_caption(&self) -> Option<String> {
        let parts: Vec<&str> = [
            self.iso_sensitivity.as_deref(),
            self.shutter_speed_time.as_deref(),
            self.f_number.as_deref(),
            self.lens_zoom_35mm_still_camera_equivalent.as_deref(),
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

/// 在所有 IFD 中查找指定 tag 的首个条目（拍摄参数通常位于 Exif 子 IFD 中）。
fn find_value<'a>(exif: &'a Exif, tag: ExifTag) -> Option<&'a EntryValue> {
    exif.iter()
        .find(|e| e.tag.tag() == Some(tag))
        .map(|e| e.value)
}

/// 从 [`Exif`] 中提取拍摄参数。
pub fn extract_capture_info(exif: &Exif) -> CaptureInfo {
    let get = |tag| find_value(exif, tag);

    CaptureInfo {
        iso: get(ExifTag::ISOSpeedRatings).and_then(format_iso),
        shutter_speed: get(ExifTag::ExposureTime).and_then(format_shutter_speed),
        aperture: get(ExifTag::FNumber).and_then(format_aperture),
        focal_length: get(ExifTag::FocalLength).and_then(format_focal_length),
        focal_length_35mm: get(ExifTag::FocalLengthIn35mmFilm).and_then(format_focal_length_35mm),
    }
}

pub async fn dump_exif(
    parser: &mut MediaParser,
    image_path: &(impl AsRef<Path> + ?Sized),
) -> Result<Option<Exif>> {
    let ms = MediaSource::open(image_path)?;
    let exif = match ms.kind() {
        MediaKind::Image => {
            let iter = parser.parse_exif(ms)?;
            Some(iter.into())
        }
        _ => None,
    };
    Ok(exif)
}

#[cfg(test)]
mod tests {
    use nom_exif::MediaParser;
    use crate::photo::dump_exif;

    #[tokio::test]
    async fn test_dump_exif() {
        let path = "./test_images/DSC_4587.jpg";
        let mut parser = MediaParser::new();
        dbg!(dump_exif(&mut parser, path).await.unwrap());
    }
}
