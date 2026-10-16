//! GPUI 位图适配；解码、合成、导出和缩放由 backend::image 负责。
mod layers;
use crate::backend::image::{self as backend_image, RasterImage};
pub use crate::backend::image::{
    PREVIEW_DEFAULT_MAX_EDGE, PreviewJob, THUMBNAIL_MAX_EDGE, export_colour_gainmap, export_gainmap,
};
use crate::backend::rotation::Rotation;
#[cfg(test)]
use crate::backend::{
    media::ExifInfo,
    watermark::{TextGroup, WatermarkParams},
};
use anyhow::Result;
use gpui_kit::RenderImage;
use image::{Frame, ImageBuffer, Rgba};
pub use layers::LayeredPreview;
pub(crate) use layers::PreviewLayerCache;
#[cfg(test)]
use std::path::PathBuf;
use std::{path::Path, sync::Arc};

pub fn render_preview(job: &PreviewJob) -> Result<Arc<RenderImage>> {
    to_gpui(backend_image::render_preview(job)?)
}
pub fn render_thumbnail(path: &Path) -> Result<Arc<RenderImage>> {
    to_gpui(backend_image::render_thumbnail(path)?)
}
pub fn render_gainmap_preview(path: &Path, show: bool) -> Result<Option<Arc<RenderImage>>> {
    backend_image::render_gainmap_preview(path, show)?
        .map(to_gpui)
        .transpose()
}
pub fn render_colour_gainmap_preview(
    path: &Path,
    rotation: Rotation,
) -> Result<(Arc<RenderImage>, Arc<RenderImage>)> {
    let (base, gainmap) = backend_image::render_colour_gainmap_preview(path, rotation)?;
    Ok((to_gpui(base)?, to_gpui(gainmap)?))
}
pub fn render_motion_photo_cover(
    ffmpeg: &Path,
    path: &Path,
    start: std::time::Duration,
    end: std::time::Duration,
    cover_time: std::time::Duration,
    rotation: Rotation,
) -> Result<Arc<RenderImage>> {
    to_gpui(backend_image::render_motion_photo_cover(
        ffmpeg, path, start, end, cover_time, rotation,
    )?)
}
pub(crate) fn render_logo_thumbnail(bytes: &[u8]) -> Result<Arc<RenderImage>> {
    to_gpui(backend_image::render_logo_thumbnail(bytes)?)
}
fn to_gpui(image: RasterImage) -> Result<Arc<RenderImage>> {
    let (w, h) = (image.width(), image.height());
    let mut pixels = image.into_pixels();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer =
        ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(w, h, pixels).expect("后端保证 RGBA 像素尺寸");
    Ok(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}
#[cfg(test)]
pub(crate) struct RenderedPreview {
    pub image: Arc<RenderImage>,
    pub text_regions: Vec<crate::backend::render::text::TextGroupRegion>,
}
#[cfg(test)]
pub(crate) fn render_preview_with_regions(job: &PreviewJob) -> Result<RenderedPreview> {
    let rendered = backend_image::render_preview_with_regions(job)?;
    Ok(RenderedPreview {
        image: to_gpui(rendered.image)?,
        text_regions: rendered.text_regions,
    })
}

#[cfg(test)]
mod preview_region_tests {
    use super::*;
    use crate::backend::image::load_scaled;
    use crate::backend::watermark::{Placement, TextAlign, TextDirection};

    #[test]
    fn text_regions_match_visible_pixels_after_placement_padding_and_rotation() {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/DSC_4587.jpg"
        ));
        let exif = ExifInfo::read(&path).unwrap();
        for position in [
            Placement::Up,
            Placement::Right,
            Placement::Bottom,
            Placement::Left,
            Placement::Center,
        ] {
            for rotation in [
                Rotation::None,
                Rotation::Clockwise90,
                Rotation::HalfTurn,
                Rotation::CounterClockwise90,
            ] {
                let mut group = TextGroup {
                    position,
                    align: TextAlign::Right,
                    padding: 0.08,
                    direction: if matches!(position, Placement::Left | Placement::Right) {
                        TextDirection::Vertical
                    } else {
                        TextDirection::Horizontal
                    },
                    ..Default::default()
                };
                group.text.template = vec!["{Logo} Lumen".into()];
                group.text.text_params.truncate(1);
                group.text.text_params[0].size = 0.04;
                let mut job = PreviewJob {
                    path: path.clone(),
                    exif: Some(exif.clone()),
                    params: WatermarkParams {
                        border_ratio: (0.2, 0.2, 0.2, 0.2),
                        solid_background: true,
                        shadow_size: 0.0,
                        rotation,
                        ..Default::default()
                    },
                    text_groups: vec![group],
                    max_edge: 400,
                };
                let base = load_scaled(&path, job.max_edge).unwrap();
                let layer = job.text_groups[0]
                    .render_text(&exif, base.height() as i32, &job.params)
                    .unwrap()
                    .unwrap();
                let rendered = render_preview_with_regions(&job).unwrap();
                // 去掉文字时保留它原先占用的边框厚度，使像素差只包含文字与 Logo。
                match position {
                    Placement::Up => {
                        job.params.border_ratio.0 += layer.height() as f64 / base.height() as f64
                    }
                    Placement::Bottom => {
                        job.params.border_ratio.1 += layer.height() as f64 / base.height() as f64
                    }
                    Placement::Left => {
                        job.params.border_ratio.2 += layer.width() as f64 / base.width() as f64
                    }
                    Placement::Right => {
                        job.params.border_ratio.3 += layer.width() as f64 / base.width() as f64
                    }
                    Placement::Center => {}
                }
                job.text_groups.clear();
                let bare = render_preview(&job).unwrap();
                assert_eq!(bare.size(0), rendered.image.size(0));
                let a = rendered.image.as_bytes(0).unwrap();
                let b = bare.as_bytes(0).unwrap();
                let width = bare.size(0).width.0 as usize;
                let (mut left, mut top, mut right, mut bottom) = (i32::MAX, i32::MAX, 0, 0);
                for (ix, (a, b)) in a
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(b.as_chunks::<4>().0)
                    .enumerate()
                {
                    if a != b {
                        let (x, y) = ((ix % width) as i32, (ix / width) as i32);
                        left = left.min(x);
                        top = top.min(y);
                        right = right.max(x + 1);
                        bottom = bottom.max(y + 1);
                    }
                }
                assert_eq!(rendered.text_regions.len(), 1);
                let region = rendered.text_regions[0];
                assert_eq!(
                    (
                        region.x,
                        region.y,
                        region.x + region.width,
                        region.y + region.height
                    ),
                    (left, top, right, bottom),
                    "{position:?} / {rotation:?}"
                );
            }
        }
    }

    #[test]
    fn missing_exif_and_empty_text_do_not_create_interactive_regions() {
        let mut job = PreviewJob {
            path: concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/DSC_4587.jpg"
            )
            .into(),
            exif: None,
            params: WatermarkParams::default(),
            text_groups: vec![TextGroup::default()],
            max_edge: 400,
        };
        assert!(
            render_preview_with_regions(&job)
                .unwrap()
                .text_regions
                .is_empty()
        );
        job.exif = ExifInfo::read(&job.path).ok();
        job.text_groups[0].text.template.clear();
        assert!(
            render_preview_with_regions(&job)
                .unwrap()
                .text_regions
                .is_empty()
        );
    }
}

#[cfg(test)]
mod custom_logo_tests {
    use super::*;
    use crate::backend::watermark::{
        GroupAttachment, Placement, Text, TextGroup, TextParams, WatermarkParams,
    };
    use std::collections::BTreeMap;
    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect x="10" y="5" width="60" height="30" fill="#ffca00"/></svg>"##;
    fn logo_group(token: &str) -> TextGroup {
        TextGroup {
            text: Text {
                template: vec![token.into()],
                text_params: vec![TextParams {
                    size: 0.08,
                    ..Default::default()
                }],
            },
            ..Default::default()
        }
    }
    #[test]
    fn adjacent_custom_logo_follows_target_and_preserves_hdr_preview() {
        let params = WatermarkParams {
            custom_logos: BTreeMap::from([("自定义logo1".into(), Arc::new(SVG.to_vec()))]),
            solid_background: true,
            ..Default::default()
        };
        let mut target = logo_group("{自定义文本}");
        target.text.text_params[0].size = 0.03;
        let logo = TextGroup {
            attachment: Some(GroupAttachment {
                target: 0,
                side: Placement::Left,
                gap: 0.0,
            }),
            ..logo_group("{自定义logo1}")
        };
        let mut job = PreviewJob {
            path: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/ultra_hdr.jpg"
            )),
            exif: None,
            params: WatermarkParams {
                custom_text: "Travel".into(),
                ..params
            },
            text_groups: vec![target, logo],
            max_edge: 480,
        };
        let first = render_preview_with_regions(&job).unwrap();
        assert_eq!(first.text_regions.len(), 2);
        let [target, logo] = first.text_regions.as_slice() else {
            panic!()
        };
        assert_eq!(logo.x + logo.width, target.x);
        job.text_groups[0].position = Placement::Up;
        job.text_groups[1].attachment.as_mut().unwrap().gap = 0.02;
        let moved = render_preview_with_regions(&job).unwrap();
        assert!(moved.text_regions[1].y < first.text_regions[1].y);
        assert!(moved.text_regions[1].x + moved.text_regions[1].width < moved.text_regions[0].x);
    }
}
