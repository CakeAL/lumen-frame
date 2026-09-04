use lumen_frame::{params::WatermarkParams, photo::Photo};

#[tokio::test]
async fn test_generate_watermark() {
    let photo_path = "./test_images/DSC_4587.jpg";
    let output_path = "./test_images/watermark";
    let photo = Photo::new(photo_path).await.unwrap();
    let params = WatermarkParams {
        output_folder: Some(output_path.into()),
        aspect_ratio: Some((16.0, 9.0)),
        position: lumen_frame::params::Position::Left,
        ..Default::default()
    };
    let watermark = photo.generate_watermark(&params).unwrap();
    photo.save_image(&params, &watermark).unwrap();
}
