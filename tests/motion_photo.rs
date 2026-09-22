use std::{path::Path, time::Duration};

use lumen_frame::features::motion_photo::{
    DEFAULT_MAX_OUTPUT_SIZE, MotionPhotoOptions, detect_ffmpeg, export_motion_photo,
    inspect_motion_photo_video,
};
use nom_exif::{MediaParser, MediaSource};

/// 手动烟雾测试：使用用户提供的 AVC MP4，输出保留在仓库根目录，便于直接导入图库验证。
#[test]
#[ignore = "需要本机 /Users/cakeal/Downloads/DSC_3049.mp4，且会在仓库根目录生成 JPEG"]
fn exports_the_requested_motion_photo() {
    let source = Path::new("/Users/cakeal/Downloads/DSC_3049.mp4");
    let output = Path::new("/Users/cakeal/Downloads/DSC_3049_motion_photo.jpg");
    let ffmpeg = detect_ffmpeg().unwrap();
    let end = inspect_motion_photo_video(&ffmpeg, source)
        .unwrap()
        .duration
        .min(Duration::from_secs(10));
    let options = MotionPhotoOptions {
        ffmpeg_path: &ffmpeg,
        video_path: source,
        output_path: output,
        start: Duration::ZERO,
        end,
        cover_time: end / 2,
        jpeg_quality: 95,
        max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
    };
    export_motion_photo(&options).unwrap();

    let mut parser = MediaParser::new();
    let exif = parser
        .parse_exif(MediaSource::open(output).unwrap())
        .unwrap();
    assert!(exif.has_embedded_track());
    parser
        .parse_track(MediaSource::open(output).unwrap())
        .unwrap();
}
