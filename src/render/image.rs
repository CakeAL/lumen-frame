use crate::media::{
    ensure_vips,
    vips::{VipsImage, from_owned_ptr, image_op},
};
use vips::Result;

use crate::{
    render::canvas::Margin,
    watermark::{Placement, WatermarkParams},
};

/// 计算图片放置位置
pub fn cal_coordinates(
    margin: &Margin,
    canvas_w: i32,
    canvas_h: i32,
    img_w: i32,
    img_h: i32,
    params: &WatermarkParams,
) -> (i32, i32) {
    // 内框尺寸
    let inner_w = canvas_w - margin.left - margin.right;
    let inner_h = canvas_h - margin.top - margin.bottom;
    // 图片在内框中居中时的坐标
    let center_x = margin.left + (inner_w - img_w) / 2;
    let center_y = margin.top + (inner_h - img_h) / 2;
    match params.position {
        Placement::Center => (center_x, center_y),
        Placement::Up => (center_x, margin.top),
        Placement::Bottom => (center_x, canvas_h - margin.bottom - img_h),
        Placement::Left => (margin.left, center_y),
        Placement::Right => (canvas_w - margin.right - img_w, center_y),
    }
}

/// 为图片添加圆角
pub fn add_round_corner(img: VipsImage, border_radius: f64) -> Result<VipsImage> {
    ensure_vips();
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let radius = (img_h as f64 * border_radius).round() as i32;
    // 使用svg生成圆角遮罩
    let svg = format!(
        r#"
            <svg xmlns="http://www.w3.org/2000/svg"
                 width="{w}"
                 height="{h}"
                 viewBox="0 0 {w} {h}">
                <rect
                    x="0"
                    y="0"
                    width="{w}"
                    height="{h}"
                    rx="{r}"
                    ry="{r}"
                    fill="white"/>
            </svg>
            "#,
        w = img_w,
        h = img_h,
        r = radius,
    );
    let svg_image = vips::VipsImage::from_buffer(svg.as_bytes())?;
    let mask = unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(svg_image.as_ptr()))? };

    // SVG 的 RGB 在半透明边缘仍是纯白，必须取 alpha 才能保留圆角抗锯齿。
    let mask = if mask.bands() > 1 {
        image_op(|out| unsafe {
            vips_sys::vips_extract_band(
                mask.as_ptr(),
                out,
                mask.bands() as i32 - 1,
                std::ptr::null::<i8>(),
            )
        })?
    } else {
        mask
    };

    // 原图已有透明度时与圆角覆盖率相乘，不能用圆角遮罩覆盖它。
    let has_alpha = unsafe { vips_sys::vips_image_hasalpha(img.as_ptr()) } != 0;
    let mask = if has_alpha {
        let alpha = alpha_mask(&img)?;
        let multiplied = image_op(|out| unsafe {
            vips_sys::vips_multiply(alpha.as_ptr(), mask.as_ptr(), out, std::ptr::null::<i8>())
        })?;
        image_op(|out| unsafe {
            vips_sys::vips_linear(
                multiplied.as_ptr(),
                out,
                [1.0 / 255.0].as_ptr(),
                [0.0].as_ptr(),
                1,
                c"uchar".as_ptr(),
                1_i32,
                std::ptr::null::<i8>(),
            )
        })?
    } else {
        mask
    };
    let colour = if has_alpha {
        image_op(|out| unsafe {
            vips_sys::vips_extract_band(
                img.as_ptr(),
                out,
                0,
                c"n".as_ptr(),
                img.bands() as i32 - 1,
                std::ptr::null::<i8>(),
            )
        })?
    } else {
        img
    };
    image_op(|out| unsafe {
        vips_sys::vips_bandjoin2(colour.as_ptr(), mask.as_ptr(), out, std::ptr::null::<i8>())
    })
}

/// 获取照片的真实覆盖率，供投影使用；无 alpha 的照片视为完全不透明。
pub(crate) fn alpha_mask(img: &VipsImage) -> Result<VipsImage> {
    if unsafe { vips_sys::vips_image_hasalpha(img.as_ptr()) } != 0 {
        image_op(|out| unsafe {
            vips_sys::vips_extract_band(
                img.as_ptr(),
                out,
                img.bands() as i32 - 1,
                std::ptr::null::<i8>(),
            )
        })
    } else {
        unsafe {
            from_owned_ptr(vips_sys::vips_image_new_from_image(
                img.as_ptr(),
                [255.0].as_ptr(),
                1,
            ))
        }
    }
}
