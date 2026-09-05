use lumen_frame::{params::WatermarkParams, photo::Photo};

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
    let params = WatermarkParams {
        output_folder: Some(output_path.into()),
        aspect_ratio: Some((16.0, 9.0)),
        position: lumen_frame::params::Position::Left,
        blur_sigma: 100.0,
        background: [26, 188, 156],
        // solid_background: true,
        border_radius: 0.02,
        ..Default::default()
    };
    let watermark = photo.generate_watermark(&params).unwrap();
    photo.save_image(&params, &watermark).unwrap();
}
