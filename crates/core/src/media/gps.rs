//! 导出成片的 GPS 元数据。只修改即将编码的图像，保持源文件与其他 EXIF 字段。

use std::ffi::{CStr, CString};

use anyhow::Result;
use nom_exif::{GPSInfo, LatLng};

use super::vips::{VipsImage, ensure_vips};

pub(crate) fn write_gps(image: &mut VipsImage, gps: Option<&GPSInfo>) -> Result<()> {
    ensure_vips();
    if let Some(gps) = gps {
        set(
            image,
            "GPSLatitudeRef",
            &gps.latitude_ref.as_char().to_string(),
        )?;
        set(
            image,
            "GPSLongitudeRef",
            &gps.longitude_ref.as_char().to_string(),
        )?;
        set(image, "GPSLatitude", &rational_triplet(gps.latitude))?;
        set(image, "GPSLongitude", &rational_triplet(gps.longitude))?;
        set(image, "GPSMapDatum", "WGS-84")?;
        if let Some(value) = gps.altitude.magnitude() {
            set(
                image,
                "GPSAltitude",
                &format!("{}/{}", value.numerator(), value.denominator()),
            )?;
            set(
                image,
                "GPSAltitudeRef",
                if gps.altitude.meters().is_some_and(|m| m < 0.0) {
                    "1"
                } else {
                    "0"
                },
            )?;
        } else {
            remove(image, "GPSAltitude")?;
            remove(image, "GPSAltitudeRef")?;
        }
        if let Some(speed) = gps.speed {
            set(
                image,
                "GPSSpeed",
                &format!("{}/{}", speed.value.numerator(), speed.value.denominator()),
            )?;
            set(image, "GPSSpeedRef", &speed.unit.as_char().to_string())?;
        } else {
            remove(image, "GPSSpeed")?;
            remove(image, "GPSSpeedRef")?;
        }
    } else {
        // libvips 会在保存时将 EXIF 字段变化同步到 exif-data，包括移除已有条目。
        let fields = unsafe { vips_sys::vips_image_get_fields(image.as_ptr()) };
        if !fields.is_null() {
            let mut i = 0;
            unsafe {
                while !(*fields.add(i)).is_null() {
                    let name = CStr::from_ptr(*fields.add(i));
                    if name.to_bytes().starts_with(b"exif-ifd3-GPS") {
                        vips_sys::vips_image_remove(image.as_ptr(), name.as_ptr());
                    }
                    vips_sys::g_free((*fields.add(i)).cast());
                    i += 1;
                }
                vips_sys::g_free(fields.cast());
            }
        }
    }
    Ok(())
}

fn rational_triplet(value: LatLng) -> String {
    [value.degrees, value.minutes, value.seconds]
        .map(|v| format!("{}/{}", v.numerator(), v.denominator()))
        .join(" ")
}

fn set(image: &VipsImage, tag: &str, value: &str) -> Result<()> {
    let tag = CString::new(format!("exif-ifd3-{tag}"))?;
    let value = CString::new(value)?;
    unsafe { vips_sys::vips_image_set_string(image.as_ptr(), tag.as_ptr(), value.as_ptr()) };
    Ok(())
}

fn remove(image: &VipsImage, tag: &str) -> Result<()> {
    let tag = CString::new(format!("exif-ifd3-{tag}"))?;
    unsafe { vips_sys::vips_image_remove(image.as_ptr(), tag.as_ptr()) };
    Ok(())
}
