use chrono::Local;
use libvips::ops;
use nom_exif::ExifDateTime;

use lumen_frame::{
    params::WatermarkParams,
    photo::{ExifInfo, Photo, Rational},
    process::{
        canvas::{self, Margin},
        text::{Text, TextAlign, TextParams, render_exif_template},
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
async fn test_render_text_with_logo_mixed() {
    let photo_path = "./test_images/DSC_4587.jpg";
    let output_path = "./test_images/watermark";
    let photo = Photo::new(photo_path).await.unwrap();
    let text = Text {
        position: lumen_frame::Position::Bottom,
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
        time_format: "%Y/%m/%d".to_owned(),
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
    let (text_position, text_height) = text.cal_height(img_h);
    let margin = Margin::cal_margin(img_w, img_h, text_height, text_position, &params);
    let (canvas_w, _canvas_h) = canvas::cal_size(&margin, img_w, img_h, &params);

    let text_img = text
        .render_text(&photo.exif.as_ref().unwrap(), img_h, &params)
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
    let has_visible = pixels.chunks_exact(4).any(|p| p[3] > 0);
    assert!(
        has_visible,
        "rendered text image should not be fully transparent"
    );

    std::fs::create_dir_all(output_path).unwrap();
    ops::pngsave(&text_img, &format!("{output_path}/text.png")).unwrap();

    // 一个没有 EXIF 相机品牌（无 logo）的模板也能正常渲染出图片
    let text = Text {
        position: lumen_frame::Position::Bottom,
        template: vec!["{拍摄日期} {光圈}".to_owned()],
        text_params: vec![TextParams {
            size: 0.03,
            ..Default::default()
        }],
        time_format: "%Y/%m/%d".to_owned(),
    };
    let params = WatermarkParams::default();
    let fallback_img = text
        .render_text(&photo.exif.as_ref().unwrap(), img_h, &params)
        .unwrap()
        .expect("render_text should produce an image");
    assert!(fallback_img.get_width() > 0);
    assert!(fallback_img.get_height() > 0);
}
