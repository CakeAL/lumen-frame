use std::{fs::File, path::Path, time::Duration};

use lumen_frame::process::motion_photo::{MotionPhotoOptions, export_motion_photo};
use nom_exif::{MediaParser, MediaSource};

/// 手动烟雾测试：使用用户提供的 AVC MP4，输出保留在仓库根目录，便于直接导入图库验证。
#[test]
#[ignore = "需要本机 /Users/cakeal/Downloads/DSC_3049.mp4，且会在仓库根目录生成 JPEG"]
fn exports_the_requested_motion_photo() {
    let source = Path::new("/Users/cakeal/Downloads/DSC_3049.mp4");
    let output = Path::new("/Users/cakeal/Downloads/DSC_3049_motion_photo.jpg");
    let mut options = MotionPhotoOptions::new(source, &output);
    options.start = Duration::from_secs(0);
    options.end = mp4::read_mp4(File::open(source).unwrap())
        .unwrap()
        .duration()
        .min(Duration::from_secs(10));
    options.cover_time = options.end / 2;
    export_motion_photo(&options).unwrap();

    let mut parser = MediaParser::new();
    let exif = parser
        .parse_exif(MediaSource::open(&output).unwrap())
        .unwrap();
    assert!(exif.has_embedded_track());
    parser
        .parse_track(MediaSource::open(&output).unwrap())
        .unwrap();
}
