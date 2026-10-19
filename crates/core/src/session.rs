//! 纯业务会话：照片身份、当前配置、各照片快照和导入默认值只有一份真值。
use super::{
    export::{ExportScope, PhotoExportJob},
    import::is_supported_image,
};
use crate::{
    media::ExifInfo,
    persistence::presets::WatermarkPreset,
    watermark::{TextGroup, WatermarkParams},
    workspace::{PhotoId, PhotoWorkspace},
};
use anyhow::{Context as _, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// 克隆仅用于冻结后台操作；提交回桌面前必须检查原会话版本。
#[derive(Clone)]
pub struct Session {
    workspace: PhotoWorkspace,
    current: WatermarkPreset,
    watermarks: HashMap<PhotoId, WatermarkPreset>,
    defaults: WatermarkPreset,
    revision: u64,
    storage: super::StoragePaths,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(WatermarkPreset {
            params: WatermarkParams::default(),
            text_groups: vec![TextGroup::default()],
        })
    }
}

impl Session {
    /// 无窗口宿主从同一持久化服务恢复本机环境，不覆盖任何磁盘文件。
    pub fn from_storage(storage: super::StoragePaths) -> Result<Self> {
        let settings = crate::settings::load_at(storage.settings()?);
        let mut session = Self::default().with_storage(storage);
        session.current.params.quality = settings.jpeg_quality.clamp(1, 100);
        if let Some(folder) = settings.output_folder {
            session.current.params.output_folder = Some(folder);
        }
        session.current.params.default_font = if settings.default_font.trim().is_empty() {
            crate::watermark::DEFAULT_FONT.to_owned()
        } else {
            settings.default_font
        };
        session.current.params.custom_logos =
            crate::logos::snapshot(&crate::logos::load_in(session.storage.logos()?)?);
        session.defaults = session.current.clone();
        Ok(session)
    }

    pub fn new(current: WatermarkPreset) -> Self {
        Self {
            workspace: PhotoWorkspace::default(),
            defaults: current.clone(),
            current,
            watermarks: HashMap::new(),
            revision: 0,
            storage: super::StoragePaths::default(),
        }
    }
    pub fn with_storage(mut self, storage: super::StoragePaths) -> Self {
        self.storage = storage;
        self
    }
    pub fn storage(&self) -> &super::StoragePaths {
        &self.storage
    }
    pub fn workspace(&self) -> &PhotoWorkspace {
        &self.workspace
    }
    pub fn defaults(&self) -> &WatermarkPreset {
        &self.defaults
    }
    #[cfg(feature = "test-support")]
    pub fn stored_photo_count(&self) -> usize {
        self.watermarks.len()
    }
    #[cfg(feature = "test-support")]
    pub fn workspace_mut(&mut self) -> &mut PhotoWorkspace {
        &mut self.workspace
    }
    pub fn current(&self) -> &WatermarkPreset {
        &self.current
    }
    pub fn params(&self) -> &WatermarkParams {
        &self.current.params
    }
    pub fn params_mut(&mut self) -> &mut WatermarkParams {
        self.changed();
        &mut self.current.params
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// 本机偏好由持久化服务保存，但它的修改也会使宿主中待提交的设置快照过期。
    pub fn mark_settings_changed(&mut self) {
        self.changed();
    }
    pub fn commit_groups(&mut self, groups: Vec<TextGroup>) {
        if self.current.text_groups != groups {
            self.current.text_groups = groups;
            self.changed();
        }
        if self.workspace.is_empty() {
            self.defaults = self.current.clone();
        }
    }
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn replace_current(&mut self, mut watermark: WatermarkPreset) {
        self.environment(&mut watermark);
        self.current = watermark;
        self.changed();
    }
    fn environment(&self, watermark: &mut WatermarkPreset) {
        let params = &mut watermark.params;
        params.output_folder = self.params().output_folder.clone();
        params.default_font = self.params().default_font.clone();
        params.custom_logos = self.params().custom_logos.clone();
        params.rotation = self.params().rotation;
        params.quality = self.params().quality;
    }
    pub fn watermark_for(&self, id: PhotoId) -> Result<WatermarkPreset> {
        self.workspace.photo(id).context("照片已移除或不存在")?;
        let mut watermark = if Some(id) == self.workspace.selected_id() {
            self.current.clone()
        } else {
            self.watermarks.get(&id).unwrap_or(&self.defaults).clone()
        };
        self.environment(&mut watermark);
        Ok(watermark)
    }
    pub fn store_current(&mut self) {
        if let Some(id) = self.workspace.selected_id() {
            self.watermarks.insert(id, self.current.clone());
        } else {
            self.defaults = self.current.clone();
        }
    }
    fn restore_selected(&mut self) {
        let watermark = self
            .workspace
            .selected_id()
            .and_then(|id| self.watermarks.remove(&id))
            .unwrap_or_else(|| self.defaults.clone());
        self.replace_current(watermark);
    }
    /// UI 已在后台扫描目录；这里接受文件列表并保留 UI 的异步 EXIF 读取节奏。
    pub fn add_photos(&mut self, paths: Vec<PathBuf>) -> Vec<(PhotoId, PathBuf)> {
        if self.workspace.is_empty() {
            self.store_current();
        }
        let mut added = Vec::new();
        let mut known = self
            .workspace
            .photos()
            .iter()
            .map(|photo| {
                photo
                    .path()
                    .canonicalize()
                    .unwrap_or_else(|_| photo.path().to_path_buf())
            })
            .collect::<HashSet<_>>();
        for path in paths {
            if !is_supported_image(&path)
                || !known.insert(path.canonicalize().unwrap_or_else(|_| path.clone()))
            {
                continue;
            }
            if let Some(id) = self.workspace.add(path.clone()) {
                self.watermarks.insert(id, self.defaults.clone());
                added.push((id, path));
            }
        }
        if self.workspace.selected_id().is_none()
            && let Some((id, _)) = added.first()
        {
            self.workspace.select(*id);
            self.restore_selected();
        }
        if !added.is_empty() {
            self.changed();
        }
        added
    }
    pub fn select(&mut self, id: PhotoId) -> bool {
        if self.workspace.selected_id() == Some(id) || self.workspace.photo(id).is_none() {
            return false;
        }
        self.store_current();
        self.workspace.select(id);
        self.restore_selected();
        true
    }
    pub fn remove_selected(&mut self) -> Option<PhotoId> {
        self.remove_photo(self.workspace.selected_id()?)
    }
    pub fn remove_photo(&mut self, id: PhotoId) -> Option<PhotoId> {
        let selected = self.workspace.selected_id() == Some(id);
        self.workspace.remove(id)?;
        self.watermarks.remove(&id);
        if selected {
            self.restore_selected();
        } else {
            self.changed();
        }
        Some(id)
    }
    pub fn clear(&mut self) -> usize {
        let count = self.workspace.clear();
        self.watermarks.clear();
        if count > 0 {
            self.restore_selected();
        }
        count
    }
    pub fn set_loaded_exif(&mut self, id: PhotoId, exif: Option<ExifInfo>) -> bool {
        let changed = self
            .workspace
            .photo_mut(id)
            .is_some_and(|photo| photo.set_loaded_exif(exif));
        if changed {
            self.changed();
        }
        changed
    }
    pub fn replace_exif(&mut self, id: PhotoId, exif: ExifInfo) -> Result<()> {
        self.workspace
            .photo_mut(id)
            .context("照片已移除或不存在")?
            .replace_exif(exif);
        self.changed();
        Ok(())
    }
    /// 应用样式时保留每张照片自己的内容；空文字组也按原样复制。
    pub fn apply_to_all(&mut self, watermark: WatermarkPreset) {
        let ids = self
            .workspace
            .photos()
            .iter()
            .map(|photo| photo.id())
            .collect::<Vec<_>>();
        for id in ids {
            if Some(id) == self.workspace.selected_id() {
                continue;
            }
            let mut copy = watermark.clone();
            copy.params.custom_text = self
                .watermark_for(id)
                .expect("队列身份有效")
                .params
                .custom_text;
            self.watermarks.insert(id, copy);
        }
        self.defaults = watermark;
        self.defaults.params.custom_text.clear();
        self.changed();
    }
    pub fn apply_preset(&mut self, mut preset: WatermarkPreset, all: bool) {
        if preset.text_groups.is_empty() {
            preset.text_groups.push(TextGroup::default());
        }
        if all {
            self.apply_to_all(preset.clone());
        }
        preset.params.custom_text = self.params().custom_text.clone();
        self.replace_current(preset);
        if self.workspace.is_empty() {
            self.store_current();
        }
    }
    pub fn export_jobs(&self, scope: ExportScope) -> Vec<PhotoExportJob> {
        self.workspace
            .photos()
            .iter()
            .filter(|p| {
                matches!(scope, ExportScope::AllPhotos)
                    || Some(p.id()) == self.workspace.selected_id()
            })
            .map(|p| PhotoExportJob {
                path: p.path().to_path_buf(),
                exif: p.exif_override().cloned(),
                watermark: self.watermark_for(p.id()).expect("队列身份有效"),
            })
            .collect()
    }
}
