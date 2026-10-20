use serde::{Deserialize, Serialize};

use crate::rotation::Rotation;

/// 照片内部的八个锚点，与最终输出旋转前的照片方向一致。
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhotoAnchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl PhotoAnchor {
    /// 从最终成片中照片区域的归一化落点选择最近的锚点。
    pub fn from_normalized_point(x: f64, y: f64, rotation: Rotation) -> Self {
        let (x, y) = match rotation {
            Rotation::None => (x, y),
            Rotation::Clockwise90 => (y, 1.0 - x),
            Rotation::HalfTurn => (1.0 - x, 1.0 - y),
            Rotation::CounterClockwise90 => (1.0 - y, x),
        };
        let cell = |value: f64| {
            if value < 1.0 / 3.0 {
                0
            } else if value > 2.0 / 3.0 {
                2
            } else {
                1
            }
        };
        match (cell(x), cell(y)) {
            (0, 0) => Self::TopLeft,
            (1, 0) => Self::Top,
            (2, 0) => Self::TopRight,
            (0, 1) => Self::Left,
            (2, 1) => Self::Right,
            (0, 2) => Self::BottomLeft,
            (1, 2) => Self::Bottom,
            (2, 2) => Self::BottomRight,
            _ if (x - 0.5).abs() > (y - 0.5).abs() => {
                if x < 0.5 {
                    Self::Left
                } else {
                    Self::Right
                }
            }
            _ => {
                if y < 0.5 {
                    Self::Top
                } else {
                    Self::Bottom
                }
            }
        }
    }

    pub(crate) fn origin(self, photo: (i32, i32), extent: (i32, i32), inset: i32) -> (i32, i32) {
        let free_x = (photo.0 - extent.0).max(0);
        let free_y = (photo.1 - extent.1).max(0);
        let (dx, dy) = (inset.min(free_x / 2), inset.min(free_y / 2));
        let x = match self {
            Self::TopLeft | Self::Left | Self::BottomLeft => dx,
            Self::TopRight | Self::Right | Self::BottomRight => free_x - dx,
            _ => free_x / 2,
        };
        let y = match self {
            Self::TopLeft | Self::Top | Self::TopRight => dy,
            Self::BottomLeft | Self::Bottom | Self::BottomRight => free_y - dy,
            _ => free_y / 2,
        };
        (x, y)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhotoPlacement {
    anchor: PhotoAnchor,
    /// 四边内侧留白，占照片高度的比例。
    #[serde(default = "default_inset")]
    inset: f64,
}

fn default_inset() -> f64 {
    0.02
}

impl PhotoPlacement {
    pub fn new(anchor: PhotoAnchor) -> Self {
        Self {
            anchor,
            inset: default_inset(),
        }
    }
    pub fn anchor(&self) -> PhotoAnchor {
        self.anchor
    }
    pub fn inset(&self) -> f64 {
        if self.inset.is_finite() {
            self.inset.clamp(0.0, 0.3)
        } else {
            default_inset()
        }
    }
    pub fn set_inset(&mut self, inset: f64) {
        self.inset = inset;
        self.inset = self.inset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_drop_keeps_the_same_photo_anchor() {
        for (x, y, rotation) in [
            (0.1, 0.1, Rotation::None),
            (0.9, 0.1, Rotation::Clockwise90),
            (0.9, 0.9, Rotation::HalfTurn),
            (0.1, 0.9, Rotation::CounterClockwise90),
        ] {
            assert_eq!(
                PhotoAnchor::from_normalized_point(x, y, rotation),
                PhotoAnchor::TopLeft
            );
        }
    }
}
