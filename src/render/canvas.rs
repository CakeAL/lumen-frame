use crate::media::{
    ensure_vips,
    vips::{VipsImage, from_owned_ptr, image_op},
};
use vips::{Result, VipsBandFormat, VipsBlendMode, VipsInterpretation};

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
        Self {
            top,
            right,
            bottom,
            left,
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

/// 生成画布
pub fn new_canvas(
    canvas_w: i32,
    canvas_h: i32,
    img: &VipsImage,
    params: &WatermarkParams,
) -> Result<VipsImage> {
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let canvas = if params.solid_background {
        // 纯色背景
        let [r, g, b] = params.background;
        let background = image_op(|out| unsafe {
            vips_sys::vips_black(
                out,
                canvas_w,
                canvas_h,
                c"bands".as_ptr(),
                3_i32,
                std::ptr::null::<i8>(),
            )
        })?;
        let a = [1.0; 3];
        let b = [r as f64, g as f64, b as f64];
        image_op(|out| unsafe {
            vips_sys::vips_linear(
                background.as_ptr(),
                out,
                a.as_ptr(),
                b.as_ptr(),
                3,
                std::ptr::null::<i8>(),
            )
        })
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
        let background_img = image_op(|out| unsafe {
            vips_sys::vips_extract_area(
                scaled_img.as_ptr(),
                out,
                crop_x,
                crop_y,
                canvas_w,
                canvas_h,
                std::ptr::null::<i8>(),
            )
        })?;
        // 4. 对裁剪后的背景图片应用高斯模糊
        image_op(|out| unsafe {
            vips_sys::vips_gaussblur(
                background_img.as_ptr(),
                out,
                params.blur_sigma,
                std::ptr::null::<i8>(),
            )
        })
    }?;
    let canvas = image_op(|out| unsafe {
        vips_sys::vips_addalpha(canvas.as_ptr(), out, std::ptr::null::<i8>())
    })?;
    let canvas = rgba_srgb(&canvas, canvas_w, canvas_h)?;
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
    image_op(|out| unsafe {
        vips_sys::vips_composite2(
            canvas.as_ptr(),
            shadow.as_ptr(),
            out,
            VipsBlendMode::VIPS_BLEND_MODE_OVER,
            c"x".as_ptr(),
            img_x + x,
            c"y".as_ptr(),
            img_y + y,
            std::ptr::null::<i8>(),
        )
    })
}

/// 独立的透明阴影图层，坐标相对于照片；预览可缓存它，不必重新做高斯模糊。
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
    let margin_x = aligned_margin(img_w);
    let margin_y = aligned_margin(img_h);
    let shadow_w = img_w + margin_x * 2;
    let shadow_h = img_h + margin_y * 2;
    let mask = super::image::alpha_mask(img)?;
    let mask = image_op(|out| unsafe {
        vips_sys::vips_embed(
            mask.as_ptr(),
            out,
            margin_x,
            margin_y,
            shadow_w,
            shadow_h,
            c"extend".as_ptr(),
            vips_sys::VipsExtend::VIPS_EXTEND_BLACK,
            std::ptr::null::<i8>(),
        )
    })?;

    // 大投影只在缩小后的单通道遮罩上做浮点模糊，使工作分辨率的 σ 不超过
    // 8 像素。比全尺寸长卷积核快得多，最后用无振铃的线性插值还原尺寸。
    let mask = image_op(|out| unsafe {
        vips_sys::vips_cast(
            mask.as_ptr(),
            out,
            VipsBandFormat::VIPS_FORMAT_FLOAT,
            std::ptr::null::<i8>(),
        )
    })?;
    let mask = if shrink > 1 {
        image_op(|out| unsafe {
            vips_sys::vips_shrink(
                mask.as_ptr(),
                out,
                shrink as f64,
                shrink as f64,
                std::ptr::null::<i8>(),
            )
        })?
    } else {
        mask
    };
    // 默认 min_ampl=0.2 的核在约 1.8σ 处截断，宽阴影的角部会出现方形边界。
    // 保留高斯尾部并全程使用浮点，直到最后生成 RGBA 才量化为 uchar。
    let mask = image_op(|out| unsafe {
        vips_sys::vips_gaussblur(
            mask.as_ptr(),
            out,
            shadow_sigma / shrink as f64,
            c"min_ampl".as_ptr(),
            0.001_f64,
            c"precision".as_ptr(),
            vips_sys::VipsPrecision::VIPS_PRECISION_FLOAT,
            std::ptr::null::<i8>(),
        )
    })?;
    // 把浓度视为光学密度：alpha = 1 - exp(-density * coverage)。旧算法直接乘
    // 浓度再裁切到 255，density > 1 时会出现纯黑平台和突变的圆角轮廓。
    let transmission = image_op(|out| unsafe {
        vips_sys::vips_linear(
            mask.as_ptr(),
            out,
            [-params.shadow_density / 255.0].as_ptr(),
            [0.0].as_ptr(),
            1,
            std::ptr::null::<i8>(),
        )
    })?;
    let transmission = image_op(|out| unsafe {
        vips_sys::vips_math(
            transmission.as_ptr(),
            out,
            vips_sys::VipsOperationMath::VIPS_OPERATION_MATH_EXP,
            std::ptr::null::<i8>(),
        )
    })?;
    let shadow_alpha = image_op(|out| unsafe {
        vips_sys::vips_linear(
            transmission.as_ptr(),
            out,
            [-255.0].as_ptr(),
            [255.0].as_ptr(),
            1,
            std::ptr::null::<i8>(),
        )
    })?;
    let shadow_alpha = if shrink > 1 {
        let interpolate = vips::VipsInterpolate::new("bilinear")?;
        // 显式按像素中心对齐：某些 libvips 版本的 resize 放大固定偏移半个
        // 输入像素，会把右下角投影加深。这里把中心位移按真实倍率换算。
        let offset = 0.5 * (1.0 - 1.0 / shrink as f64);
        image_op(|out| unsafe {
            vips_sys::vips_affine(
                shadow_alpha.as_ptr(),
                out,
                shrink as f64,
                0.0_f64,
                0.0_f64,
                shrink as f64,
                c"interpolate".as_ptr(),
                interpolate.as_ptr(),
                c"idx".as_ptr(),
                offset,
                c"idy".as_ptr(),
                offset,
                c"extend".as_ptr(),
                vips_sys::VipsExtend::VIPS_EXTEND_BLACK,
                std::ptr::null::<i8>(),
            )
        })?
    } else {
        shadow_alpha
    };
    // 生成与 mask 同尺寸的黑色 RGB（3 band）。注意不能对 VipsImage 使用 `.clone()`
    // 来复制 band：该 crate 的 Clone 是浅拷贝（不增加 GObject 引用计数），而 Drop
    // 会 unref，多次 clone 会导致 double-free / use-after-free。
    let shadow_rgb = unsafe {
        from_owned_ptr(vips_sys::vips_image_new_from_image(
            shadow_alpha.as_ptr(),
            [0.0; 3].as_ptr(),
            3,
        ))?
    };
    // RGB + Alpha → RGBA
    let shadow = image_op(|out| unsafe {
        vips_sys::vips_bandjoin2(
            shadow_rgb.as_ptr(),
            shadow_alpha.as_ptr(),
            out,
            std::ptr::null::<i8>(),
        )
    })?;
    let shadow = rgba_srgb(&shadow, shadow_w, shadow_h)?;
    Ok(Some((shadow, -margin_x, -margin_y)))
}

fn rgba_srgb(image: &VipsImage, width: i32, height: i32) -> Result<VipsImage> {
    let cast = image_op(|out| unsafe {
        vips_sys::vips_cast(
            image.as_ptr(),
            out,
            VipsBandFormat::VIPS_FORMAT_UCHAR,
            std::ptr::null::<i8>(),
        )
    })?;
    image_op(|out| unsafe {
        vips_sys::vips_copy(
            cast.as_ptr(),
            out,
            c"width".as_ptr(),
            width,
            c"height".as_ptr(),
            height,
            c"bands".as_ptr(),
            4_i32,
            c"interpretation".as_ptr(),
            VipsInterpretation::VIPS_INTERPRETATION_sRGB,
            std::ptr::null::<i8>(),
        )
    })
}
