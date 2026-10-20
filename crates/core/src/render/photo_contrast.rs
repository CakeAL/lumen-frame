//! 用照片局部亮度选择自动文字颜色；只缓存小尺寸像素，不跨线程保存 libvips 句柄。

use anyhow::Result;

use crate::{
    image::{RasterImage, rasterize},
    media::{
        ensure_vips,
        vips::{VipsImage, image_op},
    },
};

pub(crate) struct PhotoContrast {
    size: (i32, i32),
    probe: RasterImage,
}

impl PhotoContrast {
    pub(crate) fn new(source: &VipsImage) -> Result<Self> {
        ensure_vips();
        let size = (source.width() as i32, source.height() as i32);
        let image = image_op(|out| unsafe {
            vips_sys::vips_colourspace(
                source.as_ptr(),
                out,
                vips::VipsInterpretation::VIPS_INTERPRETATION_sRGB,
                std::ptr::null::<i8>(),
            )
        })?;
        let edge = size.0.max(size.1);
        let image = if edge > 256 {
            image.resize(256.0 / f64::from(edge), None, None)?
        } else {
            image
        };
        Ok(Self {
            size,
            probe: rasterize(&image)?,
        })
    }

    /// 范围使用照片坐标；忽略照片外和完全透明的像素。
    pub(crate) fn text_color(
        &self,
        (x, y, w, h): (i32, i32, i32, i32),
        background: Option<[u8; 4]>,
    ) -> Option<[u8; 3]> {
        let (left, top) = (x.max(0), y.max(0));
        let (right, bottom) = ((x + w).min(self.size.0), (y + h).min(self.size.1));
        if left >= right || top >= bottom {
            return None;
        }
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let mut luminance = 0.0;
        let mut weight = 0.0;
        for row in 0..16 {
            for col in 0..16 {
                let x = f64::from(left) + (f64::from(col) + 0.5) / 16.0 * f64::from(right - left);
                let y = f64::from(top) + (f64::from(row) + 0.5) / 16.0 * f64::from(bottom - top);
                let px = (x / f64::from(self.size.0) * f64::from(self.probe.width())) as usize;
                let py = (y / f64::from(self.size.1) * f64::from(self.probe.height())) as usize;
                let ix = (py * self.probe.width() as usize + px) * 4;
                let pixel = &self.probe.pixels()[ix..ix + 4];
                let alpha = f64::from(pixel[3]) / 255.0;
                let [r, g, b] = blend_background([pixel[0], pixel[1], pixel[2]], background);
                luminance += alpha * (0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b));
                weight += alpha;
            }
        }
        if weight == 0.0 {
            return None;
        }
        let luminance = luminance / weight;
        // 比较黑、白与局部背景的对比度，不沿用外侧画布的背景颜色。
        Some(if (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) {
            [0; 3]
        } else {
            [255; 3]
        })
    }
}

/// 与 SVG / OVER 合成相同的 sRGB 通道混合，自动配色必须考虑 Alpha。
pub(super) fn blend_background(base: [u8; 3], background: Option<[u8; 4]>) -> [u8; 3] {
    let Some([r, g, b, a]) = background else {
        return base;
    };
    std::array::from_fn(|ix| {
        ((u32::from([r, g, b][ix]) * u32::from(a)
            + u32::from(base[ix]) * (255 - u32::from(a))
            + 127)
            / 255) as u8
    })
}
