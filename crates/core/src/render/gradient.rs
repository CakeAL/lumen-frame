//! 在 libvips 中按 CSS 角度投影渐变，预览与导出共享同一条惰性管线。

use vips::{Result, VipsBandFormat};

use crate::{
    media::{ensure_vips, vips::VipsImage},
    watermark::BackgroundGradient,
};

pub(super) fn render(width: i32, height: i32, gradient: &BackgroundGradient) -> Result<VipsImage> {
    ensure_vips();
    let radians = gradient.angle().to_radians();
    let (dx, dy) = (radians.sin(), -radians.cos());
    // 使用画布四角在方向向量上的投影决定渐变线长度，适配非正方形画布。
    let span = ((width - 1) as f64 * dx.abs() + (height - 1) as f64 * dy.abs()).max(1.0);
    let xyz = vips::xyz(width as u32, height as u32)?;
    let x = xyz.extract_band(0_i32)?;
    let y = xyz.extract_band(1_i32)?;
    const LUT_SIZE: usize = 65536;
    let a = [dx / span * (LUT_SIZE - 1) as f64];
    let b = [
        (0.5 - ((width - 1) as f64 * dx + (height - 1) as f64 * dy) / (2.0 * span))
            * (LUT_SIZE - 1) as f64,
    ];
    let x = x.linear(&a, &b)?;
    let a = [dy / span * (LUT_SIZE - 1) as f64];
    let y = y.linear(&a, &[0.0])?;
    let ramp = x.add(&y)?;
    let ramp = ramp.cast(VipsBandFormat::VIPS_FORMAT_USHORT)?;
    let pixels = (0..LUT_SIZE)
        .flat_map(|i| gradient.color_at(i as f64 / (LUT_SIZE - 1) as f64))
        .collect();
    let lut = VipsImage::from_memory(
        pixels,
        LUT_SIZE as u32,
        1,
        3,
        VipsBandFormat::VIPS_FORMAT_UCHAR,
    )?;
    ramp.maplut(&lut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_angles_cover_the_canvas_and_keep_intermediate_stops() {
        for (angle, first, last) in [
            (90.0, 0, 6),
            (180.0, 0, 14),
            (0.0, 14, 0),
            (270.0, 6, 0),
            (135.0, 0, 20),
        ] {
            let gradient =
                BackgroundGradient::new(angle, vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]])
                    .unwrap();
            let image = render(7, 3, &gradient).unwrap();
            let pixels = crate::media::image_write_to_memory(&image).unwrap();
            for (index, expected) in [(first, [255, 0, 0]), (last, [0, 0, 255]), (10, [0, 255, 0])]
            {
                let actual = &pixels[index * 3..index * 3 + 3];
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(&a, b)| a.abs_diff(b) <= 1),
                    "angle={angle}, {actual:?}"
                );
            }
        }
    }
}
