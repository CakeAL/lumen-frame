//! 应用级本机偏好的 TOML 持久化。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::{
    features::geolocation::MapSettings,
    watermark::{DEFAULT_FONT, DEFAULT_JPEG_QUALITY},
};

pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lumen-frame").join("settings.toml"))
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub appearance: AppearanceMode,
    /// 原生窗口毛玻璃 / Mica 效果；旧配置未设置时默认开启。
    pub window_effects: bool,
    pub light_theme: Option<String>,
    pub dark_theme: Option<String>,
    pub interface_scale: Option<f32>,
    pub preview_max_edge: Option<i32>,
    pub output_folder: Option<PathBuf>,
    /// 全部照片共用的 JPEG 导出质量。
    pub jpeg_quality: i32,
    #[serde(default = "default_font")]
    pub default_font: String,
    #[serde(default = "default_preview_background")]
    pub preview_background: [u8; 3],
    /// 内置预设分组是否收起。
    pub builtin_presets_collapsed: bool,
    /// 用户预设的显示顺序，以预设名作为稳定身份。
    pub preset_order: Vec<String>,
    /// 地图源是本机偏好，不随水印预设或照片配置切换。
    pub map: MapSettings,
    /// 逆地理编码方式、密钥与自定义字段，仅保存到本机。
    pub geocoding: crate::geocoding::GeocodingSettings,
}

pub const DEFAULT_PREVIEW_BACKGROUND: [u8; 3] = [0x9a, 0xa7, 0xb1];

fn default_preview_background() -> [u8; 3] {
    DEFAULT_PREVIEW_BACKGROUND
}

fn default_font() -> String {
    DEFAULT_FONT.to_owned()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceMode::System,
            window_effects: true,
            light_theme: None,
            dark_theme: None,
            interface_scale: None,
            preview_max_edge: None,
            output_folder: None,
            jpeg_quality: DEFAULT_JPEG_QUALITY,
            default_font: default_font(),
            preview_background: DEFAULT_PREVIEW_BACKGROUND,
            builtin_presets_collapsed: false,
            preset_order: Vec::new(),
            map: MapSettings::default(),
            geocoding: Default::default(),
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
