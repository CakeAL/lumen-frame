use crate::media::vips::{
    add_alpha, composite_over, constant_like, scale_shadow, shadow_blur, with_interpretation,
};
use crate::media::{ensure_vips, vips::VipsImage};
use vips::{Result, VipsBandFormat, VipsInterpretation};

use crate::watermark::{Placement, WatermarkParams};

/// 画布边框margin
#[derive(Debug, Copy, Clone)]
pub struct Margin {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Margin {
    pub fn cal_margin(img_w: i32, img_h: i32, params: &WatermarkParams) -> Self {
        let (top, bottom, left, right) = if params.border_equal {
            // 边框等宽，根据上边框的宽度确定所有边框宽度
            let margin = (img_h as f64 * params.border_ratio.0).round() as i32;
            (margin, margin, margin, margin)
        } else {
            (
                (img_h as f64 * params.border_ratio.0).round() as i32,
                (img_h as f64 * params.border_ratio.1).round() as i32,
                (img_w as f64 * params.border_ratio.2).round() as i32,
                (img_w as f64 * params.border_ratio.3).round() as i32,
            )
        };
        let frame = params.photo_border.pixels(img_h);
        Self {
            top: top + frame,
            right: right + frame,
            bottom: bottom + frame,
            left: left + frame,
        }
    }

    /// 在原有边框外扩出文字组所需的厚度。同一侧的多个文字组共享一条带状区域，
    /// 因此该侧只取最厚的一组，而不是把它们逐组累加。
    pub fn include_text_thickness(&mut self, position: Placement, thickness: i32) {
        match position {
            Placement::Up => self.top += thickness,
            Placement::Right => self.right += thickness,
            Placement::Bottom => self.bottom += thickness,
            Placement::Left => self.left += thickness,
            Placement::Center => {}
        }
    }
}

// 计算画布大小
pub fn cal_size(margin: &Margin, img_w: i32, img_h: i32, params: &WatermarkParams) -> (i32, i32) {
    let mut canvas_h = img_h + margin.top + margin.bottom;
    let mut canvas_w = img_w + margin.left + margin.right;
    if let Some(aspect_ratio) = params.aspect_ratio {
        let new_h = (canvas_w as f64 / aspect_ratio.0 * aspect_ratio.1).ceil() as i32;
        if new_h < canvas_h {
            canvas_w = (canvas_h as f64 / aspect_ratio.1 * aspect_ratio.0).ceil() as i32;
        } else {
            canvas_h = new_h;
        }
    }
    (canvas_w, canvas_h)
}

/// 生成画布
pub fn new_canvas(
    canvas_w: i32,
    canvas_h: i32,
    img: &VipsImage,
    params: &WatermarkParams,
) -> Result<VipsImage> {
    ensure_vips();
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let canvas = if params.gradient_background {
        super::gradient::render(canvas_w, canvas_h, &params.gradient)
    } else if params.solid_background {
        // 纯色背景
        let [r, g, b] = params.background;
        let background = vips::black(canvas_w as u32, canvas_h as u32)?;
        background.linear(&[1.0; 3], &[r as f64, g as f64, b as f64])
    } else {
        // 模糊背景
        // 1. 计算缩放比例，使原图完全覆盖画布 (Cover 模式)
        let scale = f64::max(
            canvas_w as f64 / img_w as f64,
            canvas_h as f64 / img_h as f64,
        );
        // 2. 等比缩放原图
        let scaled_img = img.resize(scale, None, None)?;
        // 3. 从缩放后的图片中心裁剪出画布大小（居中裁剪）
        let (scaled_w, scaled_h) = (scaled_img.width() as i32, scaled_img.height() as i32);
        let crop_x = (scaled_w - canvas_w) / 2;
        let crop_y = (scaled_h - canvas_h) / 2;
        let background_img = scaled_img.crop(crop_x, crop_y, canvas_w, canvas_h)?;
        // 4. 对裁剪后的背景图片应用高斯模糊
        background_img.gaussblur(params.blur_sigma)
    }?;
    let canvas = add_alpha(&canvas)?;
    let canvas = rgba_srgb(&canvas)?;
    let canvas = if params.background_grain
        && params.grain_strength.is_finite()
        && params.grain_strength > 0.0
    {
        let grain = super::grain::apply(&canvas, img_h, params.grain_strength)?;
        rgba_srgb(&grain)?
    } else {
        canvas
    };
    Ok(canvas)
}

/// 为画布添加图片的阴影
pub fn add_shadow(
    canvas: VipsImage,
    img: &VipsImage,
    params: &WatermarkParams,
    img_x: i32,
    img_y: i32,
) -> Result<VipsImage> {
    let Some((shadow, x, y)) = shadow_layer(img, params)? else {
        return Ok(canvas);
    };
    let (offset_x, offset_y) = shadow_offset(img.height() as i32, params);
    composite_over(&canvas, &shadow, img_x + x + offset_x, img_y + y + offset_y)
}

/// 统一使用原照片高度换算偏移，避免外边框或预览缩放改变偏移比例。
pub(crate) fn shadow_offset(photo_height: i32, params: &WatermarkParams) -> (i32, i32) {
    let (x, y) = params.shadow_direction.unit_vector();
    let distance = f64::from(photo_height) * params.shadow_offset;
    ((distance * x).round() as i32, (distance * y).round() as i32)
}

/// 独立的透明阴影图层，坐标相对于照片；预览可缓存它，不必重新做高斯模糊。
/// 偏移由共享排版层应用，调整偏移时继续复用这份居中的像素。
pub(crate) fn photo_shadow_layer(
    photo: &VipsImage,
    params: &WatermarkParams,
) -> Result<Option<(VipsImage, i32, i32)>> {
    ensure_vips();
    if params.shadow_size <= 0.0 || params.shadow_density <= 0.0 {
        return Ok(None);
    }
    if let Some((border, x, y)) =
        super::image::border_layer(photo.width() as i32, photo.height() as i32, params)?
    {
        // 投影沿外边框生成，但模糊强度仍以原照片高度计算。
        let mut shadow_params = params.clone();
        shadow_params.shadow_size *= photo.height() as f64 / border.height() as f64;
        Ok(shadow_layer(&border, &shadow_params)?.map(|(image, sx, sy)| (image, sx + x, sy + y)))
    } else {
        shadow_layer(photo, params)
    }
}

/// 按给定图像的透明轮廓生成投影。
pub(crate) fn shadow_layer(
    img: &VipsImage,
    params: &WatermarkParams,
) -> Result<Option<(VipsImage, i32, i32)>> {
    ensure_vips();
    if params.shadow_size <= 0.0 || params.shadow_density <= 0.0 {
        return Ok(None);
    }
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let shadow_sigma = img_h as f64 * params.shadow_size / 3.0;
    let mask = super::image::alpha_mask(img)?;
    // 只缩放透明轮廓，保留源图的透明区域；模糊仍按原照片高度计算。
    let mask = if params.shadow_scale != 1.0 {
        mask.resize(
            params.shadow_scale,
            None,
            Some(vips::VipsKernel::VIPS_KERNEL_LINEAR),
        )?
    } else {
        mask
    };
    let (mask_w, mask_h) = (mask.width() as i32, mask.height() as i32);
    // min_ampl=0.001 的核延伸到约 3.72σ。预留 4σ 加插值余量，避免模糊边界复制
    // 非零像素。画布大小仍由用户边框决定，投影只在最终合成时被画布裁切。
    let minimum_margin = (shadow_sigma * 4.0).ceil() as i32 + 2;
    // 用整数块平均缩小遮罩。取奇数步长，让任意奇偶尺寸都能在两侧等量补边后
    // 整除步长；采样网格于是始终关于照片中心对称，不会偏向某一个角。
    let shrink = ((shadow_sigma / 8.0).ceil().max(1.0) as i32) | 1;
    let aligned_margin = |dimension: i32| {
        let mut margin = minimum_margin;
        while (dimension + margin * 2) % shrink != 0 {
            margin += 1;
        }
        margin
    };
    let margin_x = aligned_margin(mask_w);
    let margin_y = aligned_margin(mask_h);
    let shadow_w = mask_w + margin_x * 2;
    let shadow_h = mask_h + margin_y * 2;
    let mask = mask.embed(margin_x, margin_y, shadow_w, shadow_h, None)?;

    // 大投影只在缩小后的单通道遮罩上做浮点模糊，使工作分辨率的 σ 不超过
    // 8 像素。比全尺寸长卷积核快得多，最后用无振铃的线性插值还原尺寸。
    let mask = mask.cast(VipsBandFormat::VIPS_FORMAT_FLOAT)?;
    let mask = if shrink > 1 {
        mask.shrink_box(shrink as f64, shrink as f64)?
    } else {
        mask
    };
    // 默认 min_ampl=0.2 的核在约 1.8σ 处截断，宽阴影的角部会出现方形边界。
    // 保留高斯尾部并全程使用浮点，直到最后生成 RGBA 才量化为 uchar。
    let mask = shadow_blur(&mask, shadow_sigma / shrink as f64)?;
    // 把浓度视为光学密度：alpha = 1 - exp(-density * coverage)。旧算法直接乘
    // 浓度再裁切到 255，density > 1 时会出现纯黑平台和突变的圆角轮廓。
    let transmission = mask.linear1(-params.shadow_density / 255.0, 0.0)?;
    let transmission = transmission.exp()?;
    let shadow_alpha = transmission.linear1(-255.0, 255.0)?;
    let shadow_alpha = if shrink > 1 {
        scale_shadow(&shadow_alpha, shrink as f64)?
    } else {
        shadow_alpha
    };
    let shadow_rgb = constant_like(&shadow_alpha, &params.shadow_color.map(f64::from))?;
    // RGB + Alpha → RGBA
    let shadow = shadow_rgb.bandjoin2(&shadow_alpha)?;
    let shadow = rgba_srgb(&shadow)?;
    Ok(Some((
        shadow,
        (img_w - mask_w) / 2 - margin_x,
        (img_h - mask_h) / 2 - margin_y,
    )))
}

fn rgba_srgb(image: &VipsImage) -> Result<VipsImage> {
    let cast = image.cast(VipsBandFormat::VIPS_FORMAT_UCHAR)?;
    with_interpretation(&cast, VipsInterpretation::VIPS_INTERPRETATION_sRGB)
}

#[cfg(test)]
mod tests {
    use super::{Margin, cal_size};
    use crate::{render::image, watermark::WatermarkParams};

    #[test]
    fn aspect_ratio_expands_canvas_without_clipping_panorama() {
        let params = WatermarkParams {
            aspect_ratio: Some((16.0, 9.0)),
            border_equal: true,
            border_ratio: (0.05, 0.05, 0.05, 0.05),
            ..Default::default()
        };
        let (img_w, img_h) = (3000, 1000);
        let margin = Margin::cal_margin(img_w, img_h, &params);
        let (canvas_w, canvas_h) = cal_size(&margin, img_w, img_h, &params);
        let (img_x, img_y) =
            image::cal_coordinates(&margin, canvas_w, canvas_h, img_w, img_h, &params);

        assert_eq!(canvas_w, img_w + margin.left + margin.right);
        assert_eq!(canvas_h, 1744);
        assert_eq!(img_x, margin.left);
        assert!(img_y >= margin.top);
        assert!(canvas_w - img_x - img_w >= margin.right);
        assert!(canvas_h - img_y - img_h >= margin.bottom);
    }

    #[test]
    fn aspect_ratio_expands_canvas_width_for_tall_images() {
        let params = WatermarkParams {
            aspect_ratio: Some((16.0, 9.0)),
            border_ratio: (0.0, 0.0, 0.0, 0.0),
            ..Default::default()
        };
        let margin = Margin::cal_margin(1000, 2000, &params);

        assert_eq!(cal_size(&margin, 1000, 2000, &params), (3556, 2000));
    }
}
