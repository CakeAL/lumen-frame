use serde::{Deserialize, Serialize};

use super::Placement;

/// 文字组的独立边框，尺寸均以照片高度为基准。四边顺序为上、右、下、左。
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextBorder {
    enabled: bool,
    sides: [bool; 4],
    color: [u8; 3],
    width: f64,
    radius: f64,
    inset: f64,
}

impl Default for TextBorder {
    fn default() -> Self {
        Self {
            enabled: false,
            sides: [true; 4],
            color: [192; 3],
            width: 0.0008,
            radius: 0.003,
            inset: 0.004,
        }
    }
}

impl TextBorder {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn side(&self, side: Placement) -> bool {
        Self::side_index(side).is_some_and(|ix| self.sides[ix])
    }

    pub fn set_side(&mut self, side: Placement, enabled: bool) {
        if let Some(ix) = Self::side_index(side) {
            self.sides[ix] = enabled;
        }
    }

    pub fn color(&self) -> [u8; 3] {
        self.color
    }

    pub fn set_color(&mut self, color: [u8; 3]) {
        self.color = color;
    }

    pub fn width(&self) -> f64 {
        finite_ratio(self.width, 0.0001, 0.02, 0.0008)
    }

    pub fn set_width(&mut self, width: f64) {
        self.width = finite_ratio(width, 0.0001, 0.02, 0.0008);
    }

    pub fn radius(&self) -> f64 {
        finite_ratio(self.radius, 0.0, 0.05, 0.003)
    }

    pub fn set_radius(&mut self, radius: f64) {
        self.radius = finite_ratio(radius, 0.0, 0.05, 0.003);
    }

    pub fn inset(&self) -> f64 {
        finite_ratio(self.inset, 0.0, 0.05, 0.004)
    }

    pub fn set_inset(&mut self, inset: f64) {
        self.inset = finite_ratio(inset, 0.0, 0.05, 0.004);
    }

    pub(crate) fn visible(&self) -> bool {
        self.enabled && self.sides.iter().any(|enabled| *enabled)
    }

    fn side_index(side: Placement) -> Option<usize> {
        match side {
            Placement::Up => Some(0),
            Placement::Right => Some(1),
            Placement::Bottom => Some(2),
            Placement::Left => Some(3),
            Placement::Center => None,
        }
    }
}

fn finite_ratio(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}
