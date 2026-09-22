//! 应用级本机偏好的 TOML 持久化。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lumen-frame").join("settings.toml"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub appearance: AppearanceMode,
    pub light_theme: Option<String>,
    pub dark_theme: Option<String>,
    pub interface_scale: Option<f32>,
    pub preview_max_edge: Option<i32>,
    pub output_folder: Option<PathBuf>,
    #[serde(default = "default_preview_background")]
    pub preview_background: [u8; 3],
}

pub const DEFAULT_PREVIEW_BACKGROUND: [u8; 3] = [0x9a, 0xa7, 0xb1];

fn default_preview_background() -> [u8; 3] {
    DEFAULT_PREVIEW_BACKGROUND
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceMode::System,
            light_theme: None,
            dark_theme: None,
            interface_scale: None,
            preview_max_edge: None,
            output_folder: None,
            preview_background: DEFAULT_PREVIEW_BACKGROUND,
        }
    }
}

/// 文件不存在或内容损坏时退回默认值，避免偏好文件阻止应用启动。
pub fn load() -> AppSettings {
    settings_path()
        .map(|path| load_at(&path))
        .unwrap_or_default()
}

pub fn load_at(path: &Path) -> AppSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|document| toml::from_str(&document).ok())
        .unwrap_or_default()
}

pub fn save(settings: &AppSettings) -> Result<PathBuf> {
    let path = settings_path().context("找不到系统的配置目录")?;
    save_at(&path, settings)?;
    Ok(path)
}

pub fn save_at(path: &Path, settings: &AppSettings) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败：{}", parent.display()))?;
    }
    let document = toml::to_string_pretty(settings).context("序列化应用偏好失败")?;
    std::fs::write(path, document).with_context(|| format!("写入偏好失败：{}", path.display()))
}
