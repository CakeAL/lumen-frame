//! 使用实际 JPEG 保存路径验证 GPS 覆盖、清除和 Ultra HDR 保留。
use lumen_frame::{
    features::geolocation::Location,
    gainmap,
    media::{ExifInfo, load_base_image},
    photo::Photo,
    watermark::WatermarkParams,
};
use std::path::PathBuf;

struct Output(PathBuf);
impl Output {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("lumen-frame-gps-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn gps_override_and_removal_preserve_hdr_and_original_file() {
    let source = PathBuf::from("test_images/ultra_hdr.jpg");
    let original_bytes = std::fs::read(&source).unwrap();
    let mut photo = Photo::open_blocking(&source).unwrap();
    let original = photo.exif.clone().unwrap();
    assert!(original.gps_info.is_some());
    let output = Output::new("hdr");
    let params = WatermarkParams {
        output_folder: Some(output.0.clone()),
        solid_background: true,
        ..Default::default()
    };
    let base = load_base_image(&source)
        .unwrap()
        .resize(0.08, None, None)
        .unwrap();
    let composed = Photo::compose_watermark(base, photo.exif.as_ref(), &params, &[]).unwrap();
    assert!(gainmap::get_gainmap(&composed).is_some());
    let target = output.0.join("ultra_hdr_watermark.jpg");

    for location in [
        Location::new(-33.8688, 151.2093).unwrap(),
        Location::new(40.7128, -74.0060).unwrap(),
    ] {
        photo.exif.as_mut().unwrap().gps_info = Some(location.to_gps().unwrap());
        photo.save_image(&params, &composed).unwrap();
        let result = ExifInfo::read(&target).unwrap();
        let gps = result.gps_info.unwrap();
        assert!(
            (gps.latitude_decimal().unwrap() - location.latitude()).abs() < 0.00001,
            "{gps:?}"
        );
        assert!(
            (gps.longitude_decimal().unwrap() - location.longitude()).abs() < 0.00001,
            "{gps:?}"
        );
        assert_eq!(result.make, original.make);
        assert_eq!(result.model, original.model);
        assert!(gainmap::get_gainmap(&load_base_image(&target).unwrap()).is_some());
    }
    photo.exif.as_mut().unwrap().gps_info = None;
    photo.save_image(&params, &composed).unwrap();
    assert!(ExifInfo::read(&target).unwrap().gps_info.is_none());
    assert!(gainmap::get_gainmap(&load_base_image(&target).unwrap()).is_some());
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
}

#[test]
fn adds_gps_when_image_has_no_exif() {
    // 新建的小图没有源 EXIF，写入 GPS 时应让 libvips 创建 GPS IFD。
    lumen_frame::media::ensure_vips();
    let output = Output::new("new");
    let image = vips::VipsImage::from_memory(
        vec![255; 32 * 24 * 4],
        32,
        24,
        4,
        vips::VipsBandFormat::VIPS_FORMAT_UCHAR,
    )
    .unwrap();
    let location = Location::new(22.5429, 114.0596).unwrap();
    let photo = Photo {
        path: "plain.png".into(),
        is_motion_photo: false,
        is_ultra_hdr_photo: false,
        exif: Some(ExifInfo {
            gps_info: Some(location.to_gps().unwrap()),
            ..Default::default()
        }),
    };
    let params = WatermarkParams {
        output_folder: Some(output.0.clone()),
        ..Default::default()
    };
    photo.save_image(&params, &image).unwrap();
    let exif = ExifInfo::read(&output.0.join("plain_watermark.jpg")).unwrap();
    assert!(
        (exif.gps_info.unwrap().latitude_decimal().unwrap() - location.latitude()).abs() < 0.00001
    );
}
