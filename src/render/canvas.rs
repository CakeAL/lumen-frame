use crate::media::vips::{VipsImage, from_owned_ptr, image_op};
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
        let new_h = (canvas_w as f64 / aspect_ratio.0 * aspect_ratio.1).round() as i32;
        if new_h < canvas_w {
            canvas_w = (canvas_h as f64 / aspect_ratio.1 * aspect_ratio.0).round() as i32;
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
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let shadow_size = (img_h as f64 * params.shadow_size).round() as i32;
    let shadow_sigma = shadow_size as f64 / 3.0;
    // Gaussian blur 需要足够的外围空间
    let shadow_margin = (shadow_sigma * 3.0).ceil() as i32;
    let shadow_w = img_w + shadow_margin * 2;
    let shadow_h = img_h + shadow_margin * 2;
    // 创建阴影mask
    let shadow_mask = {
        let radius = (img_h as f64 * params.border_radius).round() as i32;
        let svg = format!(
            r#"
            <svg xmlns="http://www.w3.org/2000/svg"
                 width="{shadow_w}"
                 height="{shadow_h}"
                 viewBox="0 0 {shadow_w} {shadow_h}">
                <rect
                    x="{margin}"
                    y="{margin}"
                    width="{img_w}"
                    height="{img_h}"
                    rx="{radius}"
                    ry="{radius}"
                    fill="white"/>
            </svg>
            "#,
            shadow_w = shadow_w,
            shadow_h = shadow_h,
            margin = shadow_margin,
            img_w = img_w,
            img_h = img_h,
            radius = radius,
        );

        let svg_image = vips::VipsImage::from_buffer(svg.as_bytes())?;
        unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(svg_image.as_ptr()))? }
    };

    // 确保 mask 为单通道
    let shadow_mask = if shadow_mask.bands() > 1 {
        image_op(|out| unsafe {
            vips_sys::vips_extract_band(shadow_mask.as_ptr(), out, 0, std::ptr::null::<i8>())
        })?
    } else {
        shadow_mask
    };
    // 对 mask 进行高斯模糊
    let shadow_mask = image_op(|out| unsafe {
        vips_sys::vips_gaussblur(
            shadow_mask.as_ptr(),
            out,
            shadow_sigma,
            std::ptr::null::<i8>(),
        )
    })?;
    // 生成与 mask 同尺寸的黑色 RGB（3 band）。注意不能对 VipsImage 使用 `.clone()`
    // 来复制 band：该 crate 的 Clone 是浅拷贝（不增加 GObject 引用计数），而 Drop
    // 会 unref，多次 clone 会导致 double-free / use-after-free。
    let shadow_rgb = unsafe {
        from_owned_ptr(vips_sys::vips_image_new_from_image(
            shadow_mask.as_ptr(),
            [0.0; 3].as_ptr(),
            3,
        ))?
    };
    // 根据 opacity 调整 Alpha（保持 uchar，避免 linear 默认输出 float 导致 composite2 崩溃）
    let a = [params.shadow_density];
    let b = [0.0];
    let shadow_alpha = image_op(|out| unsafe {
        vips_sys::vips_linear(
            shadow_mask.as_ptr(),
            out,
            a.as_ptr(),
            b.as_ptr(),
            1,
            c"uchar".as_ptr(),
            1_i32,
            std::ptr::null::<i8>(),
        )
    })?;
    // RGB + Alpha → RGBA
    let shadow = image_op(|out| unsafe {
        vips_sys::vips_bandjoin2(
            shadow_rgb.as_ptr(),
            shadow_alpha.as_ptr(),
            out,
            std::ptr::null::<i8>(),
        )
    })?;
    // 确保 shadow 与 canvas 一样是 uchar/SRGB，避免 band 格式不一致导致 composite2 崩溃。
    let shadow = rgba_srgb(&shadow, shadow_w, shadow_h)?;
    let shadow_x = img_x - shadow_margin;
    let shadow_y = img_y - shadow_margin;

    image_op(|out| unsafe {
        vips_sys::vips_composite2(
            canvas.as_ptr(),
            shadow.as_ptr(),
            out,
            VipsBlendMode::VIPS_BLEND_MODE_OVER,
            c"x".as_ptr(),
            shadow_x,
            c"y".as_ptr(),
            shadow_y,
            std::ptr::null::<i8>(),
        )
    })
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
