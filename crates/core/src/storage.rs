//! 存储位置由宿主注入；命令不能通过预设名称改变存储根目录。
use crate::persistence::{logos, presets, settings};
use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct StoragePaths {
    settings: Option<PathBuf>,
    presets: Option<PathBuf>,
    logos: Option<PathBuf>,
}
impl Default for StoragePaths {
    fn default() -> Self {
        Self {
            settings: settings::settings_path(),
            presets: presets::directory(),
            logos: logos::directory(),
        }
    }
}
impl StoragePaths {
    pub fn in_directory(root: impl AsRef<Path>) -> Self {
        Self {
            settings: Some(root.as_ref().join("settings.toml")),
            presets: Some(root.as_ref().join("presets")),
            logos: Some(root.as_ref().join("logos")),
        }
    }
    pub fn settings(&self) -> Result<&Path> {
        self.settings.as_deref().context("找不到设置目录")
    }
    pub fn presets(&self) -> Result<&Path> {
        self.presets.as_deref().context("找不到预设目录")
    }
    pub fn logos(&self) -> Result<&Path> {
        self.logos.as_deref().context("找不到 Logo 素材目录")
    }
}
