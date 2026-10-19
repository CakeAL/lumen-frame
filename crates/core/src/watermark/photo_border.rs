use serde::{Deserialize, Serialize};

/// 照片外扩的等宽边框。宽度以照片高度为基准，色卡序号在目标照片上重新求值。
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoBorder {
    enabled: bool,
    width: f64,
    color: [u8; 3],
    palette_rank: Option<usize>,
}

impl Default for PhotoBorder {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 0.005,
            color: [255; 3],
            palette_rank: None,
        }
    }
}

impl PhotoBorder {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn width(&self) -> f64 {
        if self.width.is_finite() {
            self.width.clamp(0.001, 0.05)
        } else {
            0.005
        }
    }

    pub fn set_width(&mut self, width: f64) {
        self.width = width;
        self.width = self.width();
    }

    pub fn color(&self) -> [u8; 3] {
        self.color
    }

    /// 手动改色后使用固定颜色。
    pub fn set_color(&mut self, color: [u8; 3]) {
        self.color = color;
        self.palette_rank = None;
    }

    pub fn palette_rank(&self) -> Option<usize> {
        self.palette_rank
    }

    pub fn select_palette(&mut self, rank: usize, color: [u8; 3]) {
        self.palette_rank = Some(rank);
        self.color = color;
    }

    pub(crate) fn resolve_color(&mut self, color: [u8; 3]) {
        self.color = color;
    }

    pub(crate) fn pixels(&self, photo_height: i32) -> i32 {
        if self.enabled {
            (photo_height as f64 * self.width()).round().max(1.0) as i32
        } else {
            0
        }
    }
}
