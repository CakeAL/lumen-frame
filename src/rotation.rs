//! 照片与视频输出共用的 90° 旋转状态。

use anyhow::Result;
use libvips::{VipsImage, ops};
use serde::{Deserialize, Serialize};

use crate::gainmap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rotation {
    #[default]
    None,
    Clockwise90,
    HalfTurn,
    CounterClockwise90,
}

impl Rotation {
    pub const fn next(self) -> Self {
        match self {
            Self::None => Self::Clockwise90,
            Self::Clockwise90 => Self::HalfTurn,
            Self::HalfTurn => Self::CounterClockwise90,
            Self::CounterClockwise90 => Self::None,
        }
    }

    pub const fn degrees(self) -> u16 {
        match self {
            Self::None => 0,
            Self::Clockwise90 => 90,
            Self::HalfTurn => 180,
            Self::CounterClockwise90 => 270,
        }
    }

    pub const fn ffmpeg_filter(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Clockwise90 => Some("transpose=clock"),
            Self::HalfTurn => Some("hflip,vflip"),
            Self::CounterClockwise90 => Some("transpose=cclock"),
        }
    }
}

pub fn apply_to_image(image: &VipsImage, rotation: Rotation) -> Result<VipsImage> {
    let angle = match rotation {
        Rotation::None => ops::Angle::D0,
        Rotation::Clockwise90 => ops::Angle::D90,
        Rotation::HalfTurn => ops::Angle::D180,
        Rotation::CounterClockwise90 => ops::Angle::D270,
    };
    Ok(ops::rot(image, angle)?)
}

/// 旋转已经完成排版与合成的成片，并让内嵌 Ultra HDR gain map 保持同一方向。
pub fn apply_to_output(image: &VipsImage, rotation: Rotation) -> Result<VipsImage> {
    if rotation == Rotation::None {
        return Ok(ops::copy(image)?);
    }

    let rotated_gainmap = gainmap::get_gainmap(image)
        .map(|gainmap| {
            let gainmap = VipsImage::image_copy_memory(ops::copy(&gainmap)?)?;
            apply_to_image(&gainmap, rotation)
        })
        .transpose()?;
    // JPEG loader 常以顺序访问模式工作，而最终 rot 会倒序或跨行请求上游像素，部分
    // 相机会因此报 `out of order read`。先按原方向把完整成片求值到内存，再旋转这份
    // 内存图；排版语义不变，同时不再要求 JPEG 解码器随机访问。
    let materialized = VipsImage::image_copy_memory(ops::copy(image)?)?;
    let mut rotated = apply_to_image(&materialized, rotation)?;
    if let Some(rotated_gainmap) = rotated_gainmap.as_ref() {
        gainmap::set_gainmap(&mut rotated, rotated_gainmap);
    }
    Ok(rotated)
}

#[cfg(test)]
mod tests {
    use super::Rotation;

    #[test]
    fn rotation_cycles_through_four_quarter_turns() {
        let rotation = Rotation::None.next().next().next().next();
        assert_eq!(rotation, Rotation::None);
    }

    #[test]
    fn ffmpeg_filters_match_the_selected_turn() {
        assert_eq!(Rotation::None.ffmpeg_filter(), None);
        assert_eq!(
            Rotation::Clockwise90.ffmpeg_filter(),
            Some("transpose=clock")
        );
        assert_eq!(Rotation::HalfTurn.ffmpeg_filter(), Some("hflip,vflip"));
        assert_eq!(
            Rotation::CounterClockwise90.ffmpeg_filter(),
            Some("transpose=cclock")
        );
    }
}
