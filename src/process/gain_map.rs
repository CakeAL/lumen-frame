use libvips::{Result, VipsImage, ops};

/// 取得 VipsImage 的底层指针。
///
/// `libvips` crate 目前没有公开 gain map API，这里利用 `VipsImage` 是单字段指针
/// struct 的布局来拿到原始 `*mut bindings::VipsImage`。
unsafe fn image_raw(img: &VipsImage) -> *mut libvips::bindings::VipsImage {
    unsafe { *(img as *const VipsImage as *const *mut libvips::bindings::VipsImage) }
}

/// 如果图片带 Ultra HDR gain map，返回它。
pub fn get_gainmap(img: &VipsImage) -> Option<VipsImage> {
    unsafe {
        let raw = image_raw(img);
        let gm = libvips::bindings::vips_image_get_gainmap(raw);
        if gm.is_null() {
            // 没有 gain map 时 libvips 可能留下 error buffer，清掉避免影响后续操作。
            libvips::bindings::vips_error_clear();
            return None;
        }
        // 还原成包装类型。gain map 可能是 1 通道（BW），也可能是 3 通道（YUV 4:4:4 的
        // MPF Gain map，例如 DSC_4587），都视为真实 gain map。
        Some(std::mem::transmute::<
            *mut libvips::bindings::VipsImage,
            VipsImage,
        >(gm))
    }
}

/// 把已经构造好的 gain map 写回图片。
pub fn set_gainmap(img: &mut VipsImage, gainmap: &VipsImage) {
    unsafe {
        let raw = image_raw(img);
        let gm = image_raw(gainmap);
        let name = std::ffi::CString::new("gainmap").unwrap();
        libvips::bindings::vips_image_set_image(raw, name.as_ptr(), gm);
    }
}

/// 更新 base 上的 gainmap-scale-factor 元数据。
pub fn set_scale_factor(img: &mut VipsImage, scale: f64) {
    unsafe {
        let raw = image_raw(img);
        let name = std::ffi::CString::new("gainmap-scale-factor").unwrap();
        libvips::bindings::vips_image_set_double(raw, name.as_ptr(), scale);
    }
}

/// 构造一个覆盖整个水印画布、但只在中间照片区域保留原始 gain map 的新 gain map。
///
/// 四周填 0（对应 boost=1，即不提升亮度），避免水印边框/背景被 Ultra HDR 提亮。
pub fn make_watermark_gainmap(
    original_gainmap: &VipsImage,
    canvas_w: i32,
    canvas_h: i32,
    img_x: i32,
    img_y: i32,
    img_w: i32,
    img_h: i32,
    scale_factor: f64,
    border_radius: f64,
) -> Result<VipsImage> {
    let gm_bands = original_gainmap.get_bands().max(1);
    let gm_w = original_gainmap.get_width();
    let gm_h = original_gainmap.get_height();

    let gw = ((canvas_w as f64) / scale_factor).ceil() as i32;
    let gh = ((canvas_h as f64) / scale_factor).ceil() as i32;

    // 四周 neutral = 0（boost = 1），不增加亮度。
    // 背景的 band 数跟随 gain map（可能是 1 通道，也可能是 DSC 这种 3 通道 YUV）。
    let mut background = ops::black_with_opts(gw, gh, &ops::BlackOptions { bands: gm_bands })?;

    if gm_w > 0 && gm_h > 0 {
        let target_w = ((img_w as f64) / scale_factor).round() as i32;
        let target_h = ((img_h as f64) / scale_factor).round() as i32;
        let scaled = if (gm_w, gm_h) != (target_w, target_h) {
            ops::resize_with_opts(
                original_gainmap,
                target_w as f64 / gm_w as f64,
                &ops::ResizeOptions {
                    vscale: target_h as f64 / gm_h as f64,
                    ..Default::default()
                },
            )?
        } else {
            ops::copy(original_gainmap)?
        };
        // 如果照片有圆角，gain map 也要做同样的圆角，否则圆角处会被提亮。
        let scaled = if border_radius > 0.0 {
            let radius = ((img_h as f64 * border_radius) / scale_factor).round() as i32;
            let mask = rounded_rect_mask(target_w, target_h, radius)?;
            let zero =
                ops::black_with_opts(target_w, target_h, &ops::BlackOptions { bands: gm_bands })?;
            ops::ifthenelse(&mask, &scaled, &zero)?
        } else {
            scaled
        };
        let gx = ((img_x as f64) / scale_factor).round() as i32;
        let gy = ((img_y as f64) / scale_factor).round() as i32;
        background = ops::insert(&background, &scaled, gx, gy)?;
    }

    let interp = original_gainmap
        .get_interpretation()
        .unwrap_or(ops::Interpretation::BW);
    let background = ops::cast(&background, ops::BandFormat::Uchar)?;
    Ok(ops::copy_with_opts(
        &background,
        &ops::CopyOptions {
            width: gw,
            height: gh,
            bands: gm_bands,
            interpretation: interp,
            ..Default::default()
        },
    )?)
}

/// 生成一张白色圆角矩形 mask（1 通道 uchar），用于给 gain map 打圆角。
fn rounded_rect_mask(w: i32, h: i32, radius: i32) -> Result<VipsImage> {
    let svg = format!(
        r#"
        <svg xmlns="http://www.w3.org/2000/svg"
             width="{w}"
             height="{h}"
             viewBox="0 0 {w} {h}">
            <rect x="0" y="0" width="{w}" height="{h}"
                  rx="{radius}" ry="{radius}" fill="white"/>
        </svg>
        "#,
        w = w,
        h = h,
        radius = radius.max(0),
    );
    let mask = ops::svgload_buffer(svg.as_bytes())?;
    if mask.get_bands() > 1 {
        ops::extract_band(&mask, 0)
    } else {
        Ok(mask)
    }
}
