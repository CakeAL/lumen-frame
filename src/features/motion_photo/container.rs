//! JPEG Motion Photo 的 APP1/XMP 容器封装。

use anyhow::{Result, ensure};

pub(super) fn insert_motion_photo_xmp(
    mut jpeg: Vec<u8>,
    video_size: usize,
    timestamp: u64,
) -> Result<Vec<u8>> {
    ensure!(jpeg.starts_with(&[0xff, 0xd8]), "封面不是有效 JPEG");
    const EXIF: &[u8] = b"Exif\0\0MM\0*\0\0\0\x08\0\0\0\0\0\0\0\0";
    let xmp = format!(
        "http://ns.adobe.com/xap/1.0/\0<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:GCamera=\"http://ns.google.com/photos/1.0/camera/\" xmlns:Container=\"http://ns.google.com/photos/1.0/container/\" xmlns:Item=\"http://ns.google.com/photos/1.0/container/item/\" GCamera:MotionPhoto=\"1\" GCamera:MotionPhotoVersion=\"1\" GCamera:MotionPhotoPresentationTimestampUs=\"{timestamp}\"><Container:Directory><rdf:Seq><rdf:li rdf:parseType=\"Resource\"><Container:Item Item:Mime=\"image/jpeg\" Item:Semantic=\"Primary\"/></rdf:li><rdf:li rdf:parseType=\"Resource\"><Container:Item Item:Mime=\"video/mp4\" Item:Semantic=\"MotionPhoto\" Item:Length=\"{video_size}\" Item:Padding=\"0\"/></rdf:li></rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>"
    );
    let mut segments = app1_segment(EXIF)?;
    segments.extend(app1_segment(xmp.as_bytes())?);
    jpeg.splice(2..2, segments);
    Ok(jpeg)
}

fn app1_segment(payload: &[u8]) -> Result<Vec<u8>> {
    ensure!(payload.len() + 2 <= u16::MAX as usize, "JPEG APP1 段过长");
    let mut segment = Vec::with_capacity(payload.len() + 4);
    segment.extend_from_slice(&[0xff, 0xe1]);
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(payload);
    Ok(segment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_xmp() {
        let jpeg = insert_motion_photo_xmp(vec![0xff, 0xd8, 0xff, 0xd9], 123, 456).unwrap();
        assert!(
            jpeg.windows(b"GCamera:MotionPhoto=\"1\"".len())
                .any(|window| window == b"GCamera:MotionPhoto=\"1\"")
        );
    }
}
