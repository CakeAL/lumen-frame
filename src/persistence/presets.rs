//! 水印预设的 TOML 持久化与内置预设目录。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::watermark::{TextGroup, WatermarkParams};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatermarkPreset {
    pub params: WatermarkParams,
    pub text_groups: Vec<TextGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PresetFile {
    name: String,
    params: WatermarkParams,
    #[serde(default)]
    text_groups: Vec<TextGroup>,
}

const BUILTIN_PRESETS: &[(&str, &str)] = &[
    ("16_9", include_str!("../../assets/presets/16_9.toml")),
    (
        "基础样式",
        include_str!("../../assets/presets/基础样式.toml"),
    ),
    (
        "文字在上",
        include_str!("../../assets/presets/文字在上.toml"),
    ),
    (
        "白色边框",
        include_str!("../../assets/presets/白色边框.toml"),
    ),
    (
        "纯色背景_文字在下",
        include_str!("../../assets/presets/纯色背景_文字在下.toml"),
    ),
];

pub fn builtin() -> Result<Vec<(String, WatermarkPreset)>> {
    BUILTIN_PRESETS
        .iter()
        .map(|(name, document)| parse(document, name).map(|preset| ((*name).to_string(), preset)))
        .collect()
}

pub fn is_builtin(name: &str) -> bool {
    BUILTIN_PRESETS
        .iter()
        .any(|(builtin_name, _)| *builtin_name == name)
}

pub fn load_builtin(name: &str) -> Result<WatermarkPreset> {
    let (_, document) = BUILTIN_PRESETS
        .iter()
        .find(|(builtin_name, _)| *builtin_name == name)
        .with_context(|| format!("找不到内置预设「{name}」"))?;
    parse(document, name)
}

pub fn directory() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("lumen-frame").join("presets"))
}

pub fn list() -> Result<Vec<String>> {
    match directory() {
        Some(dir) => list_in(&dir),
        None => Ok(Vec::new()),
    }
}

pub fn list_in(dir: &Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("读取预设目录失败：{}", dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            names.push(read_name(&path).unwrap_or_else(|| stem.to_string()));
        }
    }
    names.sort();
    Ok(names)
}

pub fn save(name: &str, preset: &WatermarkPreset) -> Result<PathBuf> {
    if is_builtin(name.trim()) {
        bail!("「{}」是内置预设，请使用其他名字", name.trim());
    }
    let dir = directory().context("找不到系统的配置目录")?;
    save_in(&dir, name, preset)
}

pub fn save_in(dir: &Path, name: &str, preset: &WatermarkPreset) -> Result<PathBuf> {
    let file_name = file_name_for(name)?;
    std::fs::create_dir_all(dir).with_context(|| format!("创建预设目录失败：{}", dir.display()))?;
    let file = PresetFile {
        name: name.trim().to_string(),
        params: preset.params.clone(),
        text_groups: preset.text_groups.clone(),
    };
    let document = toml::to_string_pretty(&file).context("序列化预设失败")?;
    let path = dir.join(file_name);
    std::fs::write(&path, document).with_context(|| format!("写入预设失败：{}", path.display()))?;
    Ok(path)
}

pub fn load(name: &str) -> Result<WatermarkPreset> {
    let dir = directory().context("找不到系统的配置目录")?;
    load_in(&dir, name)
}

pub fn load_in(dir: &Path, name: &str) -> Result<WatermarkPreset> {
    let path = dir.join(file_name_for(name)?);
    let document = std::fs::read_to_string(&path)
        .with_context(|| format!("读取预设失败：{}", path.display()))?;
    parse(&document, &path.display().to_string())
}

pub fn delete(name: &str) -> Result<()> {
    if is_builtin(name) {
        bail!("内置预设不可删除");
    }
    let dir = directory().context("找不到系统的配置目录")?;
    delete_in(&dir, name)
}

pub fn delete_in(dir: &Path, name: &str) -> Result<()> {
    let path = dir.join(file_name_for(name)?);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("删除预设失败：{}", path.display()))?;
    }
    Ok(())
}

fn file_name_for(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("预设名不能为空");
    }
    let sanitized: String = trimmed
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let sanitized = sanitized.trim_matches(['.', ' ']).to_string();
    if sanitized.is_empty() {
        bail!("预设名不能只由标点组成");
    }
    Ok(format!("{sanitized}.toml"))
}

fn read_name(path: &Path) -> Option<String> {
    let document = std::fs::read_to_string(path).ok()?;
    let file: PresetFile = toml::from_str(&document).ok()?;
    Some(file.name)
}

fn parse(document: &str, source: &str) -> Result<WatermarkPreset> {
    let file: PresetFile =
        toml::from_str(document).with_context(|| format!("解析预设失败：{source}"))?;
    Ok(WatermarkPreset {
        params: file.params,
        text_groups: file.text_groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_safe_file_names() {
        assert_eq!(file_name_for("我的风格").unwrap(), "我的风格.toml");
        assert_eq!(file_name_for("  dark  ").unwrap(), "dark.toml");
        let escaped = file_name_for("../etc/passwd").unwrap();
        assert!(!escaped.contains('/'));
        assert!(!escaped.contains(".."));
        assert!(file_name_for("   ").is_err());
        assert!(file_name_for("..").is_err());
    }

    #[test]
    fn preset_round_trips_through_toml() {
        let preset = WatermarkPreset {
            params: WatermarkParams::default(),
            text_groups: vec![TextGroup::default()],
        };
        let file = PresetFile {
            name: "round trip".to_string(),
            params: preset.params.clone(),
            text_groups: preset.text_groups.clone(),
        };
        let document = toml::to_string_pretty(&file).unwrap();
        let parsed: PresetFile = toml::from_str(&document).unwrap();
        assert_eq!(parsed.name, "round trip");
        assert_eq!(parsed.params.border_ratio, preset.params.border_ratio);
        assert_eq!(parsed.text_groups, preset.text_groups);
        assert!(!document.contains("output_folder"));
    }
}
