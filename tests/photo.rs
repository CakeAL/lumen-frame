use lumen_frame::{
    params::WatermarkParams,
    photo::Photo,
    process::text::{Text, TextParams},
};

#[tokio::test]
async fn test_dump_exif() {
    let path = "./test_images/DSC_4587.jpg";
    let photo = Photo::new(&path).await.unwrap();
    dbg!(photo.exif);
}

#[tokio::test]
async fn test_generate_watermark() {
    let photo_path = "./test_images/DSC_4587.jpg";
    let output_path = "./test_images/watermark";
    let photo = Photo::new(photo_path).await.unwrap();
    let text = Text {
        position: lumen_frame::Position::Bottom,
        template: vec![
            "{Logo} {型号} {镜头型号}".to_owned(),
            "{拍摄日期} {等效焦距}mm f/{光圈} {快门}s ISO{ISO}".to_owned(),
        ],
        text_params: vec![
            TextParams {
                size: 0.03,
                bold: true,
                font: "Maple Mono NF CN".into(),
                ..Default::default()
            },
            TextParams {
                size: 0.022,
                font: "Maple Mono NF CN".into(),
                ..Default::default()
            },
        ],
        time_format: "%Y/%m/%d".to_owned(),
    };
    let params = WatermarkParams {
        output_folder: Some(output_path.into()),
        // aspect_ratio: Some((16.0, 9.0)),
        position: lumen_frame::Position::Center,
        blur_sigma: 15.0,
        background: [255, 255, 255],
        solid_background: true,
        border_radius: 0.00,
        shadow_size: 0.00,
        // border_equal: true,
        border_ratio: (0.0, 0.02, 0.05, 0.05),
        ..Default::default()
    };
    let watermark = photo.generate_watermark(&params, &text).unwrap();
    photo.save_image(&params, &watermark).unwrap();
}
