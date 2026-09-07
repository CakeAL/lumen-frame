use chrono::Local;
use libvips::ops;
use nom_exif::ExifDateTime;

use lumen_frame::{
    params::WatermarkParams,
    photo::{ExifInfo, Photo, Rational},
    process::{
        canvas::{self, Margin},
        text::{Text, TextParams, render_exif_template},
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
async fn test_render_text() {
    let photo_path = "./test_images/DSC_4587.jpg";
    let output_path = "./test_images/watermark";
    let photo = Photo::new(photo_path).await.unwrap();
    let text = Text {
        position: lumen_frame::Position::Bottom,
        template: vec![
            "{Logo} {型号} - {镜头型号}".to_owned(),
            "{拍摄日期} {等效焦距}mm f/{光圈} {快门}s ISO{ISO}".to_owned(),
        ],
        text_params: vec![
            TextParams {
                size: 0.03,
                italic: true,
                ..Default::default()
            },
            TextParams {
                size: 0.022,
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
        // solid_background: true,
        border_radius: 0.02,
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
    let text_svg = text
        .render_text(&photo.exif.unwrap(), img_h, canvas_w, &params)
        .unwrap()
        .unwrap();
    std::fs::write(&format!("{}/text.svg", output_path), &text_svg).expect("failed to save svg");
}
