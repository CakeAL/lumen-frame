//! 为文字行或整组位图绘制边框；最小尺寸只增加留白，不缩放内容。

use crate::{
    media::{
        ensure_vips,
        vips::{VipsImage, from_owned_ptr, image_op},
    },
    watermark::{Placement, TextBorder},
};
use vips::{Result, VipsBlendMode};

pub(super) fn apply(image: VipsImage, border: &TextBorder, photo_height: i32) -> Result<VipsImage> {
    ensure_vips();
    if !border.visible() {
        return Ok(image);
    }
    let scale = photo_height as f64;
    let stroke = (border.width() * scale).max(0.1);
    let inset = (border.inset() * scale + stroke).ceil() as i32;
    let [up, right_side, down, left_side] = [
        Placement::Up,
        Placement::Right,
        Placement::Bottom,
        Placement::Left,
    ]
    .map(|side| border.side(side));
    // 只为启用的边预留描边和内边距，单边分隔线不能撑开另外三个方向。
    let offsets = [up, right_side, down, left_side].map(|enabled| if enabled { inset } else { 0 });
    let [mut offset_top, offset_right, offset_bottom, mut offset_left] = offsets;
    let natural_w = image.width() as i32 + offset_left + offset_right;
    let natural_h = image.height() as i32 + offset_top + offset_bottom;
    let w = natural_w.max((border.min_width() * scale).round() as i32);
    let h = natural_h.max((border.min_height() * scale).round() as i32);
    // 最小宽高只增加留白；描边仍沿外框绘制，内容在框内居中，不能拉伸字形。
    offset_left += (w - natural_w) / 2;
    offset_top += (h - natural_h) / 2;
    let left = if left_side { stroke / 2.0 } else { 0.0 };
    let top = if up { stroke / 2.0 } else { 0.0 };
    let right = w as f64 - if right_side { stroke / 2.0 } else { 0.0 };
    let bottom = h as f64 - if down { stroke / 2.0 } else { 0.0 };
    let radius = (border.radius() * scale).min((right - left).min(bottom - top) / 2.0);
    let mut path = String::new();
    // 圆弧仅连接同时启用的相邻边；单边保留完整长度，适合 Logo 旁的分隔线。
    if up {
        let x1 = left + if left_side { radius } else { 0.0 };
        let x2 = right - if right_side { radius } else { 0.0 };
        path += &format!("M{x1},{top}H{x2}");
        if right_side && radius > 0.0 {
            path += &format!("A{radius},{radius} 0 0 1 {right},{}", top + radius);
        }
    }
    if right_side {
        let y1 = top + if up { radius } else { 0.0 };
        let y2 = bottom - if down { radius } else { 0.0 };
        path += &format!("M{right},{y1}V{y2}");
        if down && radius > 0.0 {
            path += &format!("A{radius},{radius} 0 0 1 {},{bottom}", right - radius);
        }
    }
    if down {
        let x1 = right - if right_side { radius } else { 0.0 };
        let x2 = left + if left_side { radius } else { 0.0 };
        path += &format!("M{x1},{bottom}H{x2}");
        if left_side && radius > 0.0 {
            path += &format!("A{radius},{radius} 0 0 1 {left},{}", bottom - radius);
        }
    }
    if left_side {
        let y1 = bottom - if down { radius } else { 0.0 };
        let y2 = top + if up { radius } else { 0.0 };
        path += &format!("M{left},{y1}V{y2}");
        if up && radius > 0.0 {
            path += &format!("A{radius},{radius} 0 0 1 {},{top}", left + radius);
        }
    }
    let [r, g, b] = border.color();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><path d="{path}" fill="none" stroke="rgb({r},{g},{b})" stroke-width="{stroke}" stroke-linejoin="round"/></svg>"#
    );
    let borrowed = vips::VipsImage::from_buffer(svg.as_bytes())?;
    // 立即求值，不能让延迟解码借用局部 SVG 字符串。
    let frame = unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(borrowed.as_ptr()))? };
    image_op(|out| unsafe {
        vips_sys::vips_composite2(
            frame.as_ptr(),
            image.as_ptr(),
            out,
            VipsBlendMode::VIPS_BLEND_MODE_OVER,
            c"x".as_ptr(),
            offset_left,
            c"y".as_ptr(),
            offset_top,
            std::ptr::null::<i8>(),
        )
    })
}
