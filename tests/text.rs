use chrono::Local;
use libvips::ops;
use nom_exif::{Altitude, ExifDateTime, GPSInfo, URational};

use lumen_frame::{
    params::WatermarkParams,
    photo::{ExifInfo, Photo, Rational},
    process::{
        canvas::{self, Margin},
        text::{Text, TextAlign, TextDirection, TextGroup, TextParams, render_exif_template},
    },
};

#[test]
fn test_render_exif_template() {
    let template = "{拍摄日期} {等效焦距}mm f/{光圈} {快门}s ISO{ISO} {曝光补偿}EV";
    let exif_info = ExifInfo {
        created_time: Some(ExifDateTime::Aware(Local::now().into())),
        focal_length_in35mm_film: Some(50),
        exposure_time: Some(Rational::Fraction(1, 250)),
        f_number: Some(Rational::Fraction(18, 10)),
        exposure_bias_value: Some(Rational::Fraction(-1, 3)),
        focal_length: Some(Rational::Fraction(500, 10)),
        isospeed_ratings: Some(100),
        ..Default::default()
    };
    let text = render_exif_template(template, &exif_info, "%Y/%m/%d %H:%M:%S");
    dbg!(text);
}

#[tokio::test]
async fn vertical_text_group_rotates_the_complete_group() {
    let photo = Photo::new("./test_images/DSC_4587.jpg").await.unwrap();
    let exif = photo.exif.as_ref().unwrap();
    let params = WatermarkParams::default();
    let text = Text {
        template: vec!["第一行".to_owned(), "第二行更长".to_owned()],
        text_params: vec![TextParams::default(), TextParams::default()],
    };
    let horizontal = TextGroup {
        text: text.clone(),
        ..TextGroup::default()
    }
    .render_text(exif, 1000, &params)
    .unwrap()
    .unwrap();
    let vertical = TextGroup {
        text,
        direction: TextDirection::Vertical,
        ..TextGroup::default()
    }
    .render_text(exif, 1000, &params)
    .unwrap()
    .unwrap();

    assert_eq!(vertical.get_width(), horizontal.get_height());
    assert_eq!(vertical.get_height(), horizontal.get_width());
}

#[tokio::test]
async fn side_padding_is_transparent_space_inside_the_text_image() {
    let photo = Photo::new("./test_images/DSC_4587.jpg").await.unwrap();
    let exif = photo.exif.as_ref().unwrap();
    let params = WatermarkParams::default();
    let plain = TextGroup {
        align: TextAlign::Left,
        ..TextGroup::default()
    }
    .render_text(exif, 1_000, &params)
    .unwrap()
    .unwrap();
    let padded = TextGroup {
        align: TextAlign::Left,
        padding: 0.05,
        ..TextGroup::default()
    }
    .render_text(exif, 1_000, &params)
    .unwrap()
    .unwrap();

    assert_eq!(padded.get_width(), plain.get_width() + 50);
    assert_eq!(padded.get_height(), plain.get_height());
}

#[test]
fn test_render_exif_template_missing_and_dedupe() {
    // 没有镜头型号、型号含有品牌前缀时：
    // 品牌 + 型号一起去重，缺失的 {镜头型号} 连同 “ - ” 一起被去掉。
    let exif_info = ExifInfo {
        make: Some("Xiaomi".to_owned()),
        model: Some("Xiaomi 15".to_owned()),
        lens_model: None,
        ..Default::default()
    };
    let t1 = render_exif_template("{品牌} {型号} - {镜头型号}", &exif_info, "%Y/%m/%d");
    assert_eq!(t1, "Xiaomi 15");
    let t2 = render_exif_template("{型号} - {镜头型号}", &exif_info, "%Y/%m/%d");
    assert_eq!(t2, "15");
}

#[test]
fn test_render_exif_template_gps_and_administrative_area() {
    let mut gps_info: GPSInfo = "+22.1643139-113.5570944/".parse().unwrap();
    gps_info.altitude = Altitude::BelowSeaLevel(URational::new(1234, 100));
    let exif_info = ExifInfo {
        gps_info: Some(gps_info),
        ..Default::default()
    };

    assert_eq!(
        render_exif_template("{GPS}", &exif_info, "%Y/%m/%d"),
        "22°9.86'N 113°33.43'W"
    );
    assert_eq!(
        render_exif_template("{海拔}", &exif_info, "%Y/%m/%d"),
        "-12.34"
    );

    let china_gps: GPSInfo = "+22.1643139+113.5570944/".parse().unwrap();
    let china_exif = ExifInfo {
        gps_info: Some(china_gps),
        ..Default::default()
    };
    let address = render_exif_template("{省} {市} {区}", &china_exif, "%Y/%m/%d");
    assert_eq!(address.split_whitespace().count(), 3);
}

#[tokio::test]
async fn test_render_text_with_logo_mixed() {
    let photo_path = "./test_images/DSC_4587.jpg";
    let output_path = "./test_images/watermark";
    let photo = Photo::new(photo_path).await.unwrap();
    let text = Text {
        template: vec![
            "{Logo} {型号} - {镜头型号} {品牌}".to_owned(),
            "{拍摄日期} {等效焦距}mm {实际焦距}mm f/{光圈} {快门}s ISO{ISO}".to_owned(),
        ],
        text_params: vec![
            TextParams {
                size: 0.03,
                italic: true,
                align: TextAlign::Left,
                bold: true,
                font: "Maple Mono NF CN".into(),
                ..Default::default()
            },
            TextParams {
                size: 0.022,
                align: TextAlign::Left,
                font: "Maple Mono NF CN".into(),
                ..Default::default()
            },
        ],
    };
    let params = WatermarkParams {
        output_folder: Some(output_path.into()),
        aspect_ratio: Some((16.0, 9.0)),
        position: lumen_frame::Position::Left,
        blur_sigma: 15.0,
        background: [26, 188, 156],
        border_radius: 0.02,
        solid_background: true,
        ..Default::default()
    };
    let img = ops::jpegload_with_opts(
        &photo.path.to_string_lossy(),
        &ops::JpegloadOptions {
            autorotate: true,
            ..Default::default()
        },
    )
    .unwrap();
    let (img_w, img_h) = (img.get_width(), img.get_height());
    let margin = Margin::cal_margin(img_w, img_h, &params);
    let (canvas_w, _canvas_h) = canvas::cal_size(&margin, img_w, img_h, &params);

    let text_img = text
        .render_text(photo.exif.as_ref().unwrap(), img_h, &params, "%Y/%m/%d")
        .unwrap()
        .expect("render_text should produce an image");

    let (w, h) = (text_img.get_width(), text_img.get_height());
    assert!(w > 0, "text image width must be positive");
    assert!(h > 0, "text image height must be positive");
    // 已去掉左右空白：图片宽度 = 最长一行，而不是整个画布宽度
    assert!(
        w < canvas_w,
        "text image should be trimmed to the longest line width (w={w}, canvas_w={canvas_w})"
    );

    // 确保确实有可见像素（文字或 logo）
    let pixels = text_img.image_write_to_memory();
    let has_visible = pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0);
    assert!(
        has_visible,
        "rendered text image should not be fully transparent"
    );

    std::fs::create_dir_all(output_path).unwrap();
    ops::pngsave(&text_img, &format!("{output_path}/text.png")).unwrap();

    // 一个没有 EXIF 相机品牌（无 logo）的模板也能正常渲染出图片
    let text = Text {
        template: vec!["{拍摄日期} {光圈}".to_owned()],
        text_params: vec![TextParams {
            size: 0.03,
            ..Default::default()
        }],
    };
    let params = WatermarkParams::default();
    let fallback_img = text
        .render_text(photo.exif.as_ref().unwrap(), img_h, &params, "%Y/%m/%d")
        .unwrap()
        .expect("render_text should produce an image");
    assert!(fallback_img.get_width() > 0);
    assert!(fallback_img.get_height() > 0);
}
