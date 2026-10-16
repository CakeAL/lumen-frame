//! 导出任务是冻结的业务快照，执行不依赖 UI；调用方负责调度与进度展示。
use crate::{media::ExifInfo, persistence::presets::WatermarkPreset, photo::Photo};
use anyhow::Result;
use std::path::PathBuf;
/// 启动导出时逐张冻结配置，之后切换或编辑照片不会改变正在执行的任务。
pub struct PhotoExportJob {
    pub path: PathBuf,
    pub exif: Option<ExifInfo>,
    pub watermark: WatermarkPreset,
}

#[derive(Clone, Copy)]
pub enum ExportScope {
    CurrentPhoto,
    AllPhotos,
}

impl PhotoExportJob {
    pub fn run(self) -> Result<()> {
        let mut photo = Photo::open_blocking(&self.path)?;
        if let Some(exif) = self.exif {
            photo.exif = Some(exif);
        }
        let rendered =
            photo.generate_watermark(&self.watermark.params, &self.watermark.text_groups)?;
        photo.save_image(&self.watermark.params, &rendered)
    }
}
