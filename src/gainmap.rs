use crate::media::vips::{VipsImage, from_owned_ptr, image_op};
use std::path::Path;
use vips::{Result, VipsBandFormat};

/// 取得 VipsImage 的底层指针。
///
/// 从 `vips` 包装类型取得借用指针；不可在这里释放。
fn image_raw(img: &VipsImage) -> *mut vips_sys::VipsImage {
    img.as_ptr()
}

/// 如果图片带 Ultra HDR gain map，返回它。
pub fn get_gainmap(img: &VipsImage) -> Option<VipsImage> {
    unsafe {
        let raw = image_raw(img);
        let gm = vips_sys::vips_image_get_gainmap(raw);
        if gm.is_null() {
            // 没有 gain map 时 libvips 可能留下 error buffer，清掉避免影响后续操作。
            vips_sys::vips_error_clear();
            return None;
        }
        // 还原成包装类型。gain map 可能是 1 通道（BW），也可能是 3 通道（YUV 4:4:4 的
        // MPF Gain map，例如 DSC_4587），都视为真实 gain map。
        from_owned_ptr(gm).ok()
    }
}

/// 把已经构造好的 gain map 写回图片。
pub fn set_gainmap(img: &mut VipsImage, gainmap: &VipsImage) {
    unsafe {
        let raw = image_raw(img);
        let gm = image_raw(gainmap);
        let name = std::ffi::CString::new("gainmap").unwrap();
        vips_sys::vips_image_set_image(raw, name.as_ptr(), gm);

        // jpegsave 以 gainmap-data 的存在来切换到 uhdrsave。新建的普通 JPEG 没有这项，
        // 即使已有 "gainmap" 图像也会被当作普通 JPEG 保存。uhdrsave 会优先使用上面的
        // 未压缩 gainmap，因此这里的单字节 blob 只作为保存路径标记，不会写入输出。
        let marker = 0u8;
        let name = std::ffi::CString::new("gainmap-data").unwrap();
        vips_sys::vips_image_set_blob_copy(raw, name.as_ptr(), (&marker as *const u8).cast(), 1);
    }
}

/// gain map 是否仍携带 ICC profile，用于导出前的回归测试。
pub fn has_icc_profile(img: &VipsImage) -> bool {
    unsafe {
        vips_sys::vips_image_get_typeof(
            image_raw(img),
            vips_sys::VIPS_META_ICC_NAME.as_ptr().cast(),
        ) != 0
    }
}

/// 更新 base 上的 gainmap-scale-factor 元数据。
pub fn set_scale_factor(img: &mut VipsImage, scale: f64) {
    unsafe {
        let raw = image_raw(img);
        let name = std::ffi::CString::new("gainmap-scale-factor").unwrap();
        vips_sys::vips_image_set_double(raw, name.as_ptr(), scale);
    }
}

/// 为三通道 gain map 写入与像素编码匹配的 Ultra HDR 元数据。
pub fn set_rgb_gainmap_metadata(
    img: &mut VipsImage,
    min_content_boost: [f64; 3],
    max_content_boost: [f64; 3],
) {
    // 新的彩色恢复图不应让极暗像素的异常大 gain 抬高显示门槛。将完整应用 gain map
    // 的门槛定为 2× SDR 白点，能在常见 HDR 显示器（含 macOS Preview）完整恢复彩色。
    const TARGET_DISPLAY_BOOST: f64 = 2.0;
    unsafe {
        let raw = image_raw(img);
        let set_array = |name: &str, values: &[f64; 3]| {
            let name = std::ffi::CString::new(name).unwrap();
            vips_sys::vips_image_set_array_double(raw, name.as_ptr(), values.as_ptr(), 3);
        };
        let set_double = |name: &str, value: f64| {
            let name = std::ffi::CString::new(name).unwrap();
            vips_sys::vips_image_set_double(raw, name.as_ptr(), value);
        };
        let set_int = |name: &str, value: i32| {
            let name = std::ffi::CString::new(name).unwrap();
            vips_sys::vips_image_set_int(raw, name.as_ptr(), value);
        };

        set_array("gainmap-min-content-boost", &min_content_boost);
        set_array("gainmap-max-content-boost", &max_content_boost);
        set_array("gainmap-gamma", &[1.0; 3]);
        set_array("gainmap-offset-sdr", &[1.0 / 64.0; 3]);
        set_array("gainmap-offset-hdr", &[1.0 / 64.0; 3]);
        set_double("gainmap-hdr-capacity-min", 1.0);
        set_double(
            "gainmap-hdr-capacity-max",
            max_content_boost
                .into_iter()
                .fold(1.0, f64::max)
                .min(TARGET_DISPLAY_BOOST),
        );
        set_int("gainmap-use-base-cg", 1);
    }
}

/// 为 libultrahdr 写出的 MPF 辅助 JPEG 补充 GContainer XMP 语义。
///
/// libultrahdr 的 MPF 条目只标记第二张 JPEG；部分查看器（包括 MediaInfo）还需要
/// `Item:Semantic="GainMap"` 才会把它显示为 Gain map。
pub fn mark_jpeg_gainmap(path: &Path) -> anyhow::Result<()> {
    let mut jpeg = std::fs::read(path)?;
    if jpeg
        .windows(b"Item:Semantic=\"GainMap\"".len())
        .any(|v| v == b"Item:Semantic=\"GainMap\"")
    {
        return Ok(());
    }
    let (mpf, entries, exif_end) = find_mpf_entries(&jpeg)?;
    let primary_size = read_be_u32(&jpeg, entries + 4)?;
    let secondary_size = read_be_u32(&jpeg, entries + 20)?;
    let xmp = format!(
        concat!(
            "http://ns.adobe.com/xap/1.0/\0",
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">",
            "<rdf:Description xmlns:Container=\"http://ns.google.com/photos/1.0/container/\" xmlns:Item=\"http://ns.google.com/photos/1.0/container/item/\">",
            "<Container:Directory><rdf:Seq><rdf:li rdf:parseType=\"Resource\"><Container:Item Item:Semantic=\"Primary\" Item:Mime=\"image/jpeg\"/></rdf:li>",
            "<rdf:li rdf:parseType=\"Resource\"><Container:Item Item:Semantic=\"GainMap\" Item:Mime=\"image/jpeg\" Item:Length=\"{}\"/></rdf:li>",
            "</rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>"
        ),
        secondary_size
    );
    let payload = xmp.as_bytes();
    let length = payload.len() + 2;
    anyhow::ensure!(length <= u16::MAX as usize, "GContainer XMP 过长");
    let segment_len = length + 2;
    write_be_u32(&mut jpeg, entries + 4, primary_size + segment_len as u32)?;

    let mut segment = Vec::with_capacity(segment_len);
    segment.extend_from_slice(&[0xff, 0xe1]);
    segment.extend_from_slice(&(length as u16).to_be_bytes());
    segment.extend_from_slice(payload);
    // EXIF 必须排在新增的 XMP 前面。nom-exif 3.6.1 会把遇到的第一个 APP1 当作
    // EXIF 检查，若它其实是 XMP 就直接报 JpegSegment/Tag，而不会继续向后查找。
    // libultrahdr 写出的 MPF 位于 EXIF 后面，因此把 XMP 插在 EXIF 与 MPF 之间既兼容
    // EXIF 读取，也不会改变 MPF 到辅助 JPEG 的相对偏移。
    let insertion = exif_end.unwrap_or(2);
    anyhow::ensure!(
        insertion <= mpf,
        "JPEG 的 EXIF 位于 MPF 之后，无法安全插入 XMP"
    );
    jpeg.splice(insertion..insertion, segment);
    std::fs::write(path, jpeg)?;
    Ok(())
}

fn find_mpf_entries(jpeg: &[u8]) -> anyhow::Result<(usize, usize, Option<usize>)> {
    anyhow::ensure!(jpeg.starts_with(&[0xff, 0xd8]), "不是 JPEG 文件");
    let mut offset = 2;
    let mut exif_end = None;
    while offset + 4 <= jpeg.len() {
        anyhow::ensure!(jpeg[offset] == 0xff, "JPEG 标记损坏");
        let marker = jpeg[offset + 1];
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let length = read_be_u16(jpeg, offset + 2)? as usize;
        anyhow::ensure!(
            length >= 2 && offset + 2 + length <= jpeg.len(),
            "JPEG 段长度损坏"
        );
        let payload = offset + 4;
        if marker == 0xe1 && jpeg.get(payload..payload + 6) == Some(b"Exif\0\0") {
            exif_end = Some(offset + 2 + length);
        }
        if marker == 0xe2 && jpeg.get(payload..payload + 4) == Some(b"MPF\0") {
            let tiff = payload + 4;
            anyhow::ensure!(
                jpeg.get(tiff..tiff + 8) == Some(b"MM\0*\0\0\0\x08"),
                "不支持的 MPF 字节序"
            );
            let ifd = tiff + 8;
            let tags = read_be_u16(jpeg, ifd)? as usize;
            for tag in 0..tags {
                let entry = ifd + 2 + tag * 12;
                if read_be_u16(jpeg, entry)? == 0xb002 {
                    let mp_offset = read_be_u32(jpeg, entry + 8)? as usize;
                    return Ok((offset, tiff + mp_offset, exif_end));
                }
            }
        }
        offset += 2 + length;
    }
    anyhow::bail!("JPEG 中未找到 MPF 条目")
}

fn read_be_u16(data: &[u8], at: usize) -> anyhow::Result<u16> {
    Ok(u16::from_be_bytes(
        data.get(at..at + 2)
            .ok_or_else(|| anyhow::anyhow!("MPF 越界"))?
            .try_into()?,
    ))
}

fn read_be_u32(data: &[u8], at: usize) -> anyhow::Result<u32> {
    Ok(u32::from_be_bytes(
        data.get(at..at + 4)
            .ok_or_else(|| anyhow::anyhow!("MPF 越界"))?
            .try_into()?,
    ))
}

fn write_be_u32(data: &mut [u8], at: usize, value: u32) -> anyhow::Result<()> {
    let target = data
        .get_mut(at..at + 4)
        .ok_or_else(|| anyhow::anyhow!("MPF 越界"))?;
    target.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

/// 构造一个覆盖整个水印画布、但只在中间照片区域保留原始 gain map 的新 gain map。
///
/// 四周填 0（对应 boost=1，即不提升亮度），避免水印边框/背景被 Ultra HDR 提亮。
#[allow(clippy::too_many_arguments)]
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
    let gm_bands = (original_gainmap.bands() as i32).max(1);
    let gm_w = original_gainmap.width() as i32;
    let gm_h = original_gainmap.height() as i32;

    let gw = ((canvas_w as f64) / scale_factor).ceil() as i32;
    let gh = ((canvas_h as f64) / scale_factor).ceil() as i32;

    // 背景的 band 数跟随 gain map（可能是 1 通道，也可能是 DSC 这种 3 通道 YUV）。
    // 单通道的 neutral 是 0（boost = 1）；
    // 多通道（YUV 4:4:4）的 neutral 用 128，避免色度通道为 0 导致在 macOS 上出现偏绿。
    let neutral = if gm_bands > 1 { 128.0 } else { 0.0 };
    let mut background = neutral_image(gw, gh, gm_bands, neutral)?;

    if gm_w > 0 && gm_h > 0 {
        let target_w = ((img_w as f64) / scale_factor).round() as i32;
        let target_h = ((img_h as f64) / scale_factor).round() as i32;
        let scaled = if (gm_w, gm_h) != (target_w, target_h) {
            original_gainmap.resize(
                target_w as f64 / gm_w as f64,
                Some(target_h as f64 / gm_h as f64),
                None,
            )?
        } else {
            image_op(|out| unsafe {
                vips_sys::vips_copy(original_gainmap.as_ptr(), out, std::ptr::null::<i8>())
            })?
        };
        // 如果照片有圆角，gain map 也要做同样的圆角，否则圆角处会被提亮。
        // 圆角外的 else 分支也要用 neutral 值（多通道为 0.5/128），而不是 0，
        // 否则色度通道归零会在 macOS 上把圆角处染成绿/青色。
        let scaled = if border_radius > 0.0 {
            let radius = ((img_h as f64 * border_radius) / scale_factor).round() as i32;
            let mask = rounded_rect_mask(target_w, target_h, radius)?;
            let zero = neutral_image(target_w, target_h, gm_bands, neutral)?;
            image_op(|out| unsafe {
                vips_sys::vips_ifthenelse(
                    mask.as_ptr(),
                    scaled.as_ptr(),
                    zero.as_ptr(),
                    out,
                    std::ptr::null::<i8>(),
                )
            })?
        } else {
            scaled
        };
        let gx = ((img_x as f64) / scale_factor).round() as i32;
        let gy = ((img_y as f64) / scale_factor).round() as i32;
        background = image_op(|out| unsafe {
            vips_sys::vips_insert(
                background.as_ptr(),
                scaled.as_ptr(),
                out,
                gx,
                gy,
                std::ptr::null::<i8>(),
            )
        })?;
    }

    let interp = unsafe { vips_sys::vips_image_get_interpretation(original_gainmap.as_ptr()) };
    let background = image_op(|out| unsafe {
        vips_sys::vips_cast(
            background.as_ptr(),
            out,
            VipsBandFormat::VIPS_FORMAT_UCHAR,
            std::ptr::null::<i8>(),
        )
    })?;
    image_op(|out| unsafe {
        vips_sys::vips_copy(
            background.as_ptr(),
            out,
            c"width".as_ptr(),
            gw,
            c"height".as_ptr(),
            gh,
            c"bands".as_ptr(),
            gm_bands,
            c"interpretation".as_ptr(),
            interp,
            std::ptr::null::<i8>(),
        )
    })
}

fn neutral_image(width: i32, height: i32, bands: i32, neutral: f64) -> Result<VipsImage> {
    let black = image_op(|out| unsafe {
        vips_sys::vips_black(
            out,
            width,
            height,
            c"bands".as_ptr(),
            bands,
            std::ptr::null::<i8>(),
        )
    })?;
    let a = vec![1.0; bands as usize];
    let b = vec![neutral; bands as usize];
    image_op(|out| unsafe {
        vips_sys::vips_linear(
            black.as_ptr(),
            out,
            a.as_ptr(),
            b.as_ptr(),
            bands,
            c"uchar".as_ptr(),
            1_i32,
            std::ptr::null::<i8>(),
        )
    })
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
    let svg_image = vips::VipsImage::from_buffer(svg.as_bytes())?;
    let mask = unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(svg_image.as_ptr()))? };
    if mask.bands() > 1 {
        image_op(|out| unsafe {
            vips_sys::vips_extract_band(mask.as_ptr(), out, 0, std::ptr::null::<i8>())
        })
    } else {
        Ok(mask)
    }
}
