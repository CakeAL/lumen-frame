//! 水印预设：把当前配置存成 TOML，命名保存，随时载入。
//!
//! 一个预设是「画面效果」的快照 —— 水印参数 + 文字水印。输出文件夹不在其中：它属于
//! 运行这台机器时的环境，跟着预设走只会让预设换台机器之后导出到不存在的位置（见
//! [`crate::params::WatermarkParams::output_folder`]）。
//!
//! 文件按 `<配置目录>/lumen-frame/presets/<名字>.toml` 存放，一个预设一个文件：这样
//! 用户可以直接用编辑器改、用 git 管、或者拷给别人，而不必跟一个不可读的聚合文件打交道。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{params::WatermarkParams, process::text::Text};

/// 一个命名保存的配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatermarkPreset {
    pub params: WatermarkParams,
    pub text: Text,
}

/// 预设文件里记录的元信息。
///
/// 名字同时决定文件名，两者可能因为非法字符被规整而不同，所以名字在文件里单独存一份。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PresetFile {
    /// 文件格式版本，方便以后改结构时迁移。
    version: u32,
    name: String,
    params: WatermarkParams,
    text: Text,
}

const PRESET_FORMAT_VERSION: u32 = 1;

/// 预设的存放目录。
pub fn preset_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lumen-frame").join("presets"))
}

/// 应用级偏好。
///
/// 和预设分开放：预设是「一份画面效果」，可以有很多个、可以给别人；这里是「这台机器上
/// 这个应用怎么显示」，只有一个。
pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lumen-frame").join("settings.toml"))
}

/// 界面明暗的三种选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppearanceMode {
    /// 跟随系统外观，并在系统切换时跟着变。
    #[default]
    System,
    Light,
    Dark,
}

/// 应用级偏好的内容。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub appearance: AppearanceMode,
    /// 浅色主题的名字。`None` 表示用默认的那套。
    pub light_theme: Option<String>,
    /// 深色主题的名字。`None` 表示用默认的那套。
    ///
    /// 分两个槽位是主题系统的原生模型：`Theme::apply_config` 按配色自己的 mode 写进
    /// 对应槽位，明暗切换时在这个槽位里取。所以「跟随系统」才有意义 —— 系统切到深色就
    /// 用深色槽里的那套。
    pub dark_theme: Option<String>,
    /// 界面缩放的基础字号。`None` 表示用标准档。
    ///
    /// 它落在 GPUI 全局主题的 `font_size` 上，而那个值不跨进程保留，所以必须在这里
    /// 单独记一份，否则「设置里选了宽松、重启又变回标准」。
    pub interface_scale: Option<f32>,
}

/// 读取应用偏好。文件不存在或读坏了都退回默认值 —— 一个偏好文件不该拦住启动。
pub fn load_settings() -> AppSettings {
    settings_path()
        .map(|path| load_settings_at(&path))
        .unwrap_or_default()
}

pub fn load_settings_at(path: &Path) -> AppSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|document| toml::from_str(&document).ok())
        .unwrap_or_default()
}

/// 保存应用偏好。
pub fn save_settings(settings: &AppSettings) -> Result<PathBuf> {
    let path = settings_path().context("找不到系统的配置目录")?;
    save_settings_at(&path, settings)?;
    Ok(path)
}

pub fn save_settings_at(path: &Path, settings: &AppSettings) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败：{}", parent.display()))?;
    }
    let document = toml::to_string_pretty(settings).context("序列化应用偏好失败")?;
    std::fs::write(path, document).with_context(|| format!("写入偏好失败：{}", path.display()))
}

/// 列出已保存的预设名，按名字排序。
pub fn list_presets() -> Result<Vec<String>> {
    match preset_dir() {
        Some(dir) => list_presets_in(&dir),
        None => Ok(Vec::new()),
    }
}

/// 列出指定目录里的预设名。
///
/// 目录不存在时返回空列表：没保存过不是错误。
pub fn list_presets_in(dir: &Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("读取预设目录失败：{}", dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        // 文件名就是名字；读不出内容也不该让整张列表消失。
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            names.push(read_name(&path).unwrap_or_else(|| stem.to_string()));
        }
    }
    names.sort();
    Ok(names)
}

/// 保存预设，同名覆盖。
pub fn save_preset(name: &str, preset: &WatermarkPreset) -> Result<PathBuf> {
    let dir = preset_dir().context("找不到系统的配置目录")?;
    save_preset_in(&dir, name, preset)
}

/// 保存到指定目录。默认目录只是它的一个调用点，分离出来是为了能对着临时目录测。
pub fn save_preset_in(dir: &Path, name: &str, preset: &WatermarkPreset) -> Result<PathBuf> {
    let file_name = file_name_for(name)?;
    std::fs::create_dir_all(dir).with_context(|| format!("创建预设目录失败：{}", dir.display()))?;

    let file = PresetFile {
        version: PRESET_FORMAT_VERSION,
        name: name.trim().to_string(),
        params: preset.params.clone(),
        text: preset.text.clone(),
    };
    let document = toml::to_string_pretty(&file).context("序列化预设失败")?;
    let path = dir.join(file_name);
    std::fs::write(&path, document).with_context(|| format!("写入预设失败：{}", path.display()))?;
    Ok(path)
}

/// 按名字载入预设。
pub fn load_preset(name: &str) -> Result<WatermarkPreset> {
    let dir = preset_dir().context("找不到系统的配置目录")?;
    load_preset_in(&dir, name)
}

/// 从指定目录载入预设。
pub fn load_preset_in(dir: &Path, name: &str) -> Result<WatermarkPreset> {
    let path = dir.join(file_name_for(name)?);
    let document = std::fs::read_to_string(&path)
        .with_context(|| format!("读取预设失败：{}", path.display()))?;
    let file: PresetFile =
        toml::from_str(&document).with_context(|| format!("解析预设失败：{}", path.display()))?;

    if file.version > PRESET_FORMAT_VERSION {
        bail!(
            "预设「{}」来自更新的版本（格式 {}）",
            file.name,
            file.version
        );
    }
    Ok(WatermarkPreset {
        params: file.params,
        text: file.text,
    })
}

/// 删除预设。文件不在就当作已经删掉了。
pub fn delete_preset(name: &str) -> Result<()> {
    let dir = preset_dir().context("找不到系统的配置目录")?;
    delete_preset_in(&dir, name)
}

/// 从指定目录删除预设。
pub fn delete_preset_in(dir: &Path, name: &str) -> Result<()> {
    let path = dir.join(file_name_for(name)?);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("删除预设失败：{}", path.display()))?;
    }
    Ok(())
}

/// 把用户给的名字变成一个安全的文件名。
///
/// 预设名是用户随手起的，可能带路径分隔符、`..` 或者其它文件系统不接受的字符；宁可在
/// 保存前规整，也不要让一次误输入写到目录外面去。
fn file_name_for(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("预设名不能为空");
    }

    let sanitized: String = trimmed
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let sanitized = sanitized.trim_matches(['.', ' ']).to_string();
    if sanitized.is_empty() {
        bail!("预设名不能只由标点组成");
    }
    Ok(format!("{sanitized}.toml"))
}

/// 读取文件里记录的名字，拿不到就返回 `None`（由调用方退回文件名）。
fn read_name(path: &Path) -> Option<String> {
    let document = std::fs::read_to_string(path).ok()?;
    let file: PresetFile = toml::from_str(&document).ok()?;
    Some(file.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_safe_file_names() {
        assert_eq!(file_name_for("我的风格").unwrap(), "我的风格.toml");
        assert_eq!(file_name_for("  dark  ").unwrap(), "dark.toml");

        // 关键性质是「写不出目录外面」，而不是某个具体的替换结果。
        let escaped = file_name_for("../etc/passwd").unwrap();
        assert!(!escaped.contains('/'), "{escaped}");
        assert!(!escaped.contains(".."), "{escaped}");
        assert!(escaped.ends_with(".toml"));

        assert!(file_name_for("   ").is_err());
        assert!(file_name_for("..").is_err());
    }

    #[test]
    fn preset_round_trips_through_toml() {
        let preset = WatermarkPreset {
            params: WatermarkParams::default(),
            text: Text::default(),
        };
        let file = PresetFile {
            version: PRESET_FORMAT_VERSION,
            name: "round trip".to_string(),
            params: preset.params.clone(),
            text: preset.text.clone(),
        };
        let document = toml::to_string_pretty(&file).unwrap();
        let parsed: PresetFile = toml::from_str(&document).unwrap();

        assert_eq!(parsed.name, "round trip");
        assert_eq!(parsed.params.border_ratio, preset.params.border_ratio);
        assert_eq!(parsed.text.template, preset.text.template);
        assert_eq!(parsed.text.text_params.len(), preset.text.text_params.len());
        // 输出文件夹不写进预设。反序列化会拿到 `WatermarkParams::default()` 的那份，
        // 所以载入预设的人必须显式保留当前值 —— 见 `AppView::apply_preset`。
        assert!(!document.contains("output_folder"));
    }
}
