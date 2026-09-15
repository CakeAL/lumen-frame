use lumen_frame::{
    params::WatermarkParams,
    photo::Photo,
    process::text::{Text, TextAlign, TextDirection, TextGroup, TextParams},
};

#[tokio::test]
async fn test_dump_exif() {
    let path = "./test_images/ultra_hdr.jpg";
    let photo = Photo::new(&path).await.unwrap();
    dbg!(photo.exif);
}

#[tokio::test]
async fn test_generate_watermark() {
    let photo_path = "./test_images/DSC_4587.jpg";
    // let photo_path = "./test_images/ultra_hdr.jpg";
    let output_path = "./test_images/watermark";
    let photo = Photo::new(photo_path).await.unwrap();
    // 同一侧可以放多个文字组；组级 align 决定它们贴图片的哪一端，行级 align 仍只
    // 决定每组内部各行如何对齐。
    let text_groups = vec![
        TextGroup {
            text: Text {
                template: vec!["{Logo} {型号}".to_owned()],
                text_params: vec![TextParams {
                    size: 0.03,
                    bold: true,
                    align: TextAlign::Left,
                    font: "Maple Mono NF CN".into(),
                    ..Default::default()
                }],
            },
            position: lumen_frame::Position::Up,
            align: TextAlign::Left,
            time_format: "%Y/%m/%d".to_owned(),
            ..TextGroup::default()
        },
        TextGroup {
            text: Text {
                template: vec![
                    "{拍摄日期}".to_owned(),
                    "{等效焦距}mm f/{光圈} {快门}s ISO{ISO}".to_owned(),
                ],
                text_params: vec![
                    TextParams {
                        size: 0.022,
                        align: TextAlign::Right,
                        font: "Maple Mono NF CN".into(),
                        ..Default::default()
                    },
                    TextParams {
                        size: 0.018,
                        align: TextAlign::Right,
                        font: "Maple Mono NF CN".into(),
                        ..Default::default()
                    },
                ],
            },
            position: lumen_frame::Position::Up,
            align: TextAlign::Right,
            time_format: "%Y/%m/%d".to_owned(),
            ..TextGroup::default()
        },
        TextGroup {
            text: Text {
                template: vec!["LUMEN FRAME".to_owned()],
                text_params: vec![TextParams {
                    size: 0.018,
                    font: "Maple Mono NF CN".into(),
                    ..Default::default()
                }],
            },
            position: lumen_frame::Position::Right,
            direction: TextDirection::Vertical,
            align: TextAlign::Center,
            time_format: "%Y/%m/%d".to_owned(),
        },
    ];
    let params = WatermarkParams {
        output_folder: Some(output_path.into()),
        aspect_ratio: Some((16.0, 9.0)),
        position: lumen_frame::Position::Center,
        blur_sigma: 50.0,
        background: [255, 255, 255],
        // solid_background: true,
        border_radius: 0.02,
        shadow_size: 0.06,
        border_equal: true,
        // border_ratio: (0.0, 0.02, 0.05, 0.05),
        ..Default::default()
    };
    let watermark = photo.generate_watermark(&params, &text_groups).unwrap();
    photo.save_image(&params, &watermark).unwrap();
}
