//! 与界面、图像库无关的等距多色线性渐变。角度沿用 CSS：0° 向上，90° 向右。

use std::{fmt, str::FromStr};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BackgroundGradient {
    angle: f64,
    colors: Vec<[u8; 3]>,
}

impl BackgroundGradient {
    pub fn new(angle: f64, colors: Vec<[u8; 3]>) -> Result<Self> {
        ensure!(angle.is_finite(), "渐变角度必须是有限数值");
        ensure!(!colors.is_empty(), "请至少保留一个颜色");
        Ok(Self {
            angle: angle.rem_euclid(360.0),
            colors,
        })
    }

    pub fn angle(&self) -> f64 {
        self.angle
    }
    pub fn colors(&self) -> &[[u8; 3]] {
        &self.colors
    }

    pub fn color_at(&self, fraction: f64) -> [u8; 3] {
        if self.colors.len() == 1 {
            return self.colors[0];
        }
        let position = fraction.clamp(0.0, 1.0) * (self.colors.len() - 1) as f64;
        let index = (position.floor() as usize).min(self.colors.len() - 2);
        let t = position - index as f64;
        std::array::from_fn(|channel| {
            (f64::from(self.colors[index][channel]) * (1.0 - t)
                + f64::from(self.colors[index + 1][channel]) * t)
                .round() as u8
        })
    }
}

impl Default for BackgroundGradient {
    fn default() -> Self {
        Self::new(135.0, vec![[0xb5, 0xb9, 0xff], [0x2b, 0x2c, 0x49]]).unwrap()
    }
}

impl FromStr for BackgroundGradient {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim().to_ascii_lowercase();
        let Some(body) = value
            .strip_prefix("linear-gradient(")
            .and_then(|s| s.strip_suffix(')'))
        else {
            bail!("请输入 linear-gradient(135deg,#f02fc2,#6094ea) 格式的渐变");
        };
        let mut parts = body.split(',').map(str::trim);
        let angle = parts
            .next()
            .and_then(|s| s.strip_suffix("deg"))
            .and_then(|s| s.trim().parse::<f64>().ok())
            .ok_or_else(|| anyhow::anyhow!("渐变角度请使用数字加 deg，例如 135deg"))?;
        let mut colors = Vec::new();
        for part in parts {
            let hex = part.strip_prefix('#').unwrap_or("");
            ensure!(
                matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "颜色请使用 #RGB 或 #RRGGBB 格式，色标位置暂不支持"
            );
            let hex = if hex.len() == 3 {
                hex.chars().flat_map(|c| [c, c]).collect::<String>()
            } else {
                hex.to_owned()
            };
            let number = u32::from_str_radix(&hex, 16)?;
            colors.push([(number >> 16) as u8, (number >> 8) as u8, number as u8]);
        }
        Self::new(angle, colors)
    }
}

impl fmt::Display for BackgroundGradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "linear-gradient({}deg", self.angle)?;
        for [r, g, b] in &self.colors {
            write!(f, ",#{r:02x}{g:02x}{b:02x}")?;
        }
        write!(f, ")")
    }
}

impl TryFrom<String> for BackgroundGradient {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl From<BackgroundGradient> for String {
    fn from(value: BackgroundGradient) -> Self {
        value.to_string()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PaletteTheme {
    #[default]
    Default,
    Basic,
    Colorful,
    Vivid,
    Muted,
    Light,
    Dark,
}

/// `None` 是手动颜色，`Some(rank)` 是该照片在指定 Theme 下的代表色序号。
/// 旧预设没有来源信息时，全部按固定颜色处理。
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GradientPalette {
    theme: PaletteTheme,
    /// TOML 数组不支持 null，用 -1 表示手动颜色。
    slots: Vec<i32>,
}

impl GradientPalette {
    pub fn new(theme: PaletteTheme, slots: Vec<Option<usize>>) -> Self {
        Self {
            theme,
            slots: slots
                .into_iter()
                .map(|slot| slot.and_then(|n| i32::try_from(n).ok()).unwrap_or(-1))
                .collect(),
        }
    }

    pub fn theme(&self) -> PaletteTheme {
        self.theme
    }

    pub fn slots(&self, count: usize) -> Vec<Option<usize>> {
        (0..count)
            .map(|ix| self.slots.get(ix).and_then(|&n| usize::try_from(n).ok()))
            .collect()
    }

    pub fn uses_palette(&self) -> bool {
        self.slots.iter().any(|&n| n >= 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_roundtrip_and_equal_color_stops() {
        let gradient: BackgroundGradient = "linear-gradient(-225deg, #f00, #00ff00, #00f)"
            .parse()
            .unwrap();
        assert_eq!(gradient.angle(), 135.0);
        assert_eq!(gradient.color_at(0.0), [255, 0, 0]);
        assert_eq!(gradient.color_at(0.5), [0, 255, 0]);
        assert_eq!(gradient.color_at(1.0), [0, 0, 255]);
        assert_eq!(gradient, gradient.to_string().parse().unwrap());
    }

    #[test]
    fn rejects_invalid_or_unsupported_gradients() {
        for text in [
            "linear-gradient(NaNdeg,#000,#fff)",
            "linear-gradient(90deg)",
            "linear-gradient(90deg,#ffff,#000)",
            "linear-gradient(90deg,#000 20%,#fff)",
            "red",
        ] {
            assert!(text.parse::<BackgroundGradient>().is_err(), "{text}");
        }
    }

    #[test]
    fn gradient_accepts_more_than_sixteen_stops_and_recipe_roundtrips() {
        let colors = vec![[10, 20, 30]; 32];
        let gradient = BackgroundGradient::new(135.0, colors.clone()).unwrap();
        assert_eq!(gradient.colors(), colors);
        let recipe = GradientPalette::new(PaletteTheme::Muted, vec![None, Some(0), Some(3)]);
        let restored: GradientPalette = toml::from_str(&toml::to_string(&recipe).unwrap()).unwrap();
        assert_eq!(restored, recipe);
        assert_eq!(restored.slots(4), [None, Some(0), Some(3), None]);
    }

    #[test]
    fn single_color_is_constant_and_roundtrips() {
        let gradient = BackgroundGradient::new(135.0, vec![[10, 20, 30]]).unwrap();
        for fraction in [0.0, 0.5, 1.0] {
            assert_eq!(gradient.color_at(fraction), [10, 20, 30]);
        }
        assert_eq!(gradient, gradient.to_string().parse().unwrap());
        assert!(BackgroundGradient::new(135.0, vec![]).is_err());
    }
}
