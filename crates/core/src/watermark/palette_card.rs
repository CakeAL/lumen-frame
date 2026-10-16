//! 照片色卡水印。只持久化取色序号，颜色及占比由目标照片重新解析。

use serde::{Deserialize, Serialize};

use super::{GroupAttachment, PaletteTheme, Placement, TextAlign, TextDirection};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteShape {
    #[default]
    Square,
    Circle,
    Rounded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaletteCard {
    enabled: bool,
    theme: PaletteTheme,
    selected: Vec<usize>,
    position: Placement,
    align: TextAlign,
    direction: TextDirection,
    shape: PaletteShape,
    proportional: bool,
    show_hex: bool,
    show_ratio: bool,
    size: f64,
    width: f64,
    length: f64,
    gap: f64,
    radius: f64,
    padding: f64,
    attachment: Option<GroupAttachment>,
    label_size: f64,
    label_color: [u8; 3],
    #[serde(skip)]
    swatches: Vec<([u8; 3], f64)>,
}

impl Default for PaletteCard {
    fn default() -> Self {
        Self {
            enabled: false,
            theme: PaletteTheme::Default,
            selected: (0..6).collect(),
            position: Placement::Bottom,
            align: TextAlign::Right,
            direction: TextDirection::Horizontal,
            shape: PaletteShape::Circle,
            proportional: false,
            show_hex: false,
            show_ratio: false,
            size: 0.03,
            width: 0.05,
            length: 0.35,
            gap: 0.01,
            radius: 0.006,
            padding: 0.01,
            attachment: None,
            label_size: 0.01,
            label_color: [64; 3],
            swatches: Vec::new(),
        }
    }
}

impl PaletteCard {
    pub fn attachment(&self) -> Option<&GroupAttachment> {
        self.attachment.as_ref()
    }
    pub fn set_attachment(&mut self, attachment: Option<GroupAttachment>) {
        self.attachment = attachment;
    }
    pub fn attachment_gap(&self) -> f64 {
        self.attachment
            .as_ref()
            .map(|a| finite(a.gap, 0.0, 0.0, 0.2))
            .unwrap_or(0.0)
    }
    pub fn set_attachment_gap(&mut self, gap: f64) {
        if let Some(attachment) = &mut self.attachment {
            attachment.gap = finite(gap, 0.0, 0.0, 0.2);
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn theme(&self) -> PaletteTheme {
        self.theme
    }
    pub fn set_theme(&mut self, theme: PaletteTheme) {
        self.theme = theme;
        self.swatches.clear();
    }
    pub fn position(&self) -> Placement {
        self.position
    }
    pub fn set_position(&mut self, position: Placement) {
        self.position = position;
    }
    pub fn align(&self) -> TextAlign {
        self.align
    }
    pub fn set_align(&mut self, align: TextAlign) {
        self.align = align;
    }
    pub fn direction(&self) -> TextDirection {
        self.direction
    }
    pub fn set_direction(&mut self, direction: TextDirection) {
        self.direction = direction;
    }
    pub fn shape(&self) -> PaletteShape {
        self.shape
    }
    pub fn set_shape(&mut self, shape: PaletteShape) {
        self.shape = shape;
        if shape == PaletteShape::Circle {
            self.proportional = false;
        }
    }
    pub fn proportional(&self) -> bool {
        self.proportional && self.shape != PaletteShape::Circle
    }
    pub fn set_proportional(&mut self, proportional: bool) {
        self.proportional = proportional;
    }
    pub fn show_hex(&self) -> bool {
        self.show_hex
    }
    pub fn set_show_hex(&mut self, show: bool) {
        self.show_hex = show;
    }
    pub fn show_ratio(&self) -> bool {
        self.show_ratio
    }
    pub fn set_show_ratio(&mut self, show: bool) {
        self.show_ratio = show;
    }
    pub fn label_color(&self) -> [u8; 3] {
        self.label_color
    }
    pub fn set_label_color(&mut self, color: [u8; 3]) {
        self.label_color = color;
    }
    pub fn selected(&self) -> &[usize] {
        &self.selected
    }
    pub fn set_selected(&mut self, ranks: Vec<usize>) {
        self.selected.clear();
        for rank in ranks {
            if rank < 6 && !self.selected.contains(&rank) {
                self.selected.push(rank);
            }
        }
    }
    pub fn toggle(&mut self, rank: usize) {
        let mut ranks = self.selected.clone();
        if let Some(ix) = ranks.iter().position(|r| *r == rank) {
            ranks.remove(ix);
        } else {
            ranks.push(rank);
        }
        self.set_selected(ranks);
    }

    pub fn size(&self) -> f64 {
        finite(self.size, 0.03, 0.005, 0.2)
    }
    pub fn set_size(&mut self, value: f64) {
        self.size = value;
        self.size = self.size();
    }
    pub fn width(&self) -> f64 {
        finite(self.width, 0.05, 0.005, 0.4)
    }
    pub fn set_width(&mut self, value: f64) {
        self.width = value;
        self.width = self.width();
    }
    pub fn length(&self) -> f64 {
        finite(self.length, 0.35, 0.05, 2.0)
    }
    pub fn set_length(&mut self, value: f64) {
        self.length = value;
        self.length = self.length();
    }
    pub fn gap(&self) -> f64 {
        finite(self.gap, 0.01, 0.0, 0.1)
    }
    pub fn set_gap(&mut self, value: f64) {
        self.gap = value;
        self.gap = self.gap();
    }
    pub fn radius(&self) -> f64 {
        finite(self.radius, 0.006, 0.0, 0.1)
    }
    pub fn set_radius(&mut self, value: f64) {
        self.radius = value;
        self.radius = self.radius();
    }
    pub fn padding(&self) -> f64 {
        finite(self.padding, 0.01, 0.0, 0.2)
    }
    pub fn set_padding(&mut self, value: f64) {
        self.padding = value;
        self.padding = self.padding();
    }
    pub fn label_size(&self) -> f64 {
        finite(self.label_size, 0.01, 0.005, 0.05)
    }
    pub fn set_label_size(&mut self, value: f64) {
        self.label_size = value;
        self.label_size = self.label_size();
    }

    /// 已选颜色保留点击顺序；目标照片颜色较少时略过不存在的序号，不重复补色。
    pub fn colors(&self) -> Vec<([u8; 3], f64)> {
        let mut seen = Vec::new();
        self.selected
            .iter()
            .filter_map(|rank| {
                if seen.contains(rank) {
                    return None;
                }
                seen.push(*rank);
                self.swatches.get(*rank).copied()
            })
            .collect()
    }

    pub(crate) fn resolve(&mut self, swatches: Vec<([u8; 3], f64)>) {
        self.swatches = swatches;
    }
}

fn finite(value: f64, default: f64, min: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}
