use nom_exif::{Exif, MediaKind, MediaParser, MediaSource, Result};
use std::path::Path;

pub async fn dump_exif(
    parser: &mut MediaParser,
    image_path: &(impl AsRef<Path> + ?Sized),
) -> Result<Option<Exif>> {
    let ms = MediaSource::open(image_path)?;
    let exif = match ms.kind() {
        MediaKind::Image => {
            let iter = parser.parse_exif(ms)?;
            Some(iter.into())
        }
        _ => None,
    };
    Ok(exif)
}

#[cfg(test)]
mod tests {
    use nom_exif::MediaParser;
    use crate::parse::dump_exif;

    #[tokio::test]
    async fn test_dump_exif() {
        let path = "/Users/cakeal/Downloads/DSC_0792.jpg";
        let mut parser = MediaParser::new();
        dbg!(dump_exif(&mut parser, path).await.unwrap());
    }
}
