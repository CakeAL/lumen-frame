use std::path::Path;

use lumen_frame::{
    gainmap,
    media::{ensure_vips, load_base_image},
    photo::Photo,
    rotation::Rotation,
    watermark::WatermarkParams,
};

fn thumbnail_bytes(image: &vips::VipsImage<'_>) -> Vec<u8> {
    let mut data = std::ptr::null();
    let mut length = 0;
    vips::code_to_result(unsafe {
        vips_sys::vips_image_get_blob(
            image.as_ptr(),
            c"jpeg-thumbnail-data".as_ptr(),
            &mut data,
            &mut length,
        )
    })
    .expect("导出文件应包含 EXIF 缩略图");
    assert!(length > 0);
    unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length).to_vec() }
}

fn check_export_thumbnail(source: &str, hdr: bool, rotation: Rotation) {
    ensure_vips();
    let photo = Photo::open_blocking(source).unwrap();
    let original = load_base_image(Path::new(source)).unwrap();
    let original_thumbnail = thumbnail_bytes(&original);
    let base = original
        .resize(
            768.0 / original.width().max(original.height()) as f64,
            None,
            None,
        )
        .unwrap();
    if !hdr {
        for field in [c"gainmap-data", c"gainmap"] {
            unsafe { vips_sys::vips_image_remove(base.as_ptr(), field.as_ptr()) };
        }
    }

    let output_folder = std::env::temp_dir().join(format!(
        "lumen-frame-export-thumbnail-{}-{hdr}",
        std::process::id()
    ));
    let params = WatermarkParams {
        output_folder: Some(output_folder.clone()),
        aspect_ratio: Some((16.0, 9.0)),
        border_ratio: (0.2, 0.2, 0.2, 0.2),
        solid_background: true,
        background: [220, 30, 40],
        border_radius: 0.015,
        shadow_size: 0.105,
        shadow_density: 2.0,
        rotation,
        ..Default::default()
    };
    let composed = Photo::compose_watermark(base, photo.exif.as_ref(), &params, &[]).unwrap();
    photo.save_image(&params, &composed).unwrap();

    let stem = Path::new(source).file_stem().unwrap().to_str().unwrap();
    let exported = load_base_image(&output_folder.join(format!("{stem}_watermark.jpg"))).unwrap();
    assert_eq!(gainmap::get_gainmap(&exported).is_some(), hdr);
    assert!(gainmap::has_icc_profile(&exported), "导出必须保留色彩配置");

    let bytes = thumbnail_bytes(&exported);
    assert_ne!(bytes, original_thumbnail, "不能沿用原图的缩略图");
    let thumbnail = vips::VipsImage::from_buffer(&bytes).unwrap();
    let expected_size = match rotation {
        Rotation::None => (256, 144),
        Rotation::Clockwise90 => (144, 256),
        _ => unreachable!(),
    };
    assert_eq!((thumbnail.width(), thumbnail.height()), expected_size);
    let pixels = thumbnail.write_to_memory().unwrap();
    for (actual, expected) in pixels[..3].iter().zip(params.background) {
        assert!(
            (i32::from(*actual) - i32::from(expected)).abs() < 10,
            "缩略图应包含最终成片的边框背景"
        );
    }
    let exported_exif =
        lumen_frame::media::ExifInfo::read(&output_folder.join(format!("{stem}_watermark.jpg")))
            .unwrap();
    assert_eq!(exported_exif.make, photo.exif.as_ref().unwrap().make);
    assert_eq!(exported_exif.model, photo.exif.as_ref().unwrap().model);
    std::fs::remove_dir_all(output_folder).unwrap();
}

#[test]
fn ordinary_jpeg_thumbnail_matches_the_watermarked_output() {
    check_export_thumbnail("./test_images/DSC_4587.jpg", false, Rotation::None);
}

#[test]
fn ultra_hdr_thumbnail_matches_the_watermarked_output() {
    check_export_thumbnail("./test_images/ultra_hdr.jpg", true, Rotation::Clockwise90);
}
