//! RGBA 图层到 GPUI 纹理的适配，按后端像素身份复用上传结果。
use super::{PreviewJob, to_gpui};
use crate::backend::{
    image::{RasterImage, layers as raster},
    render::text::TextGroupRegion,
};
use anyhow::Result;
use gpui_kit::{DevicePixels, RenderImage, Size, size};
use std::sync::{Arc, OnceLock};

pub(crate) struct PositionedLayer {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) x: i32,
    pub(crate) y: i32,
}
pub struct LayeredPreview {
    size: Size<DevicePixels>,
    pub(crate) layers: Vec<PositionedLayer>,
    pub(crate) text_regions: Vec<TextGroupRegion>,
    pub(crate) palette_region: Option<TextGroupRegion>,
    raster: Arc<raster::LayeredPreview>,
    flattened: OnceLock<Arc<RenderImage>>,
}
impl LayeredPreview {
    #[cfg(test)]
    pub(crate) fn is_flattened(&self) -> bool {
        self.flattened.get().is_some()
    }
    pub(crate) fn size(&self) -> Size<DevicePixels> {
        self.size
    }

    pub(crate) fn photo_region(&self) -> TextGroupRegion {
        self.raster.photo_region()
    }
    pub(crate) fn image(&self) -> &Arc<RenderImage> {
        self.flattened.get_or_init(|| {
            to_gpui((**self.raster.image()).clone()).expect("后端保证 RGBA 像素尺寸")
        })
    }
}
#[derive(Default)]
pub(crate) struct PreviewLayerCache {
    backend: raster::PreviewLayerCache,
    textures: Vec<(Arc<RasterImage>, Arc<RenderImage>)>,
    #[cfg(test)]
    builds: [usize; 5],
}
impl PreviewLayerCache {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.backend.is_empty() && self.textures.is_empty()
    }
    pub(crate) fn render(&mut self, job: &PreviewJob) -> Result<Arc<LayeredPreview>> {
        let scene = self.backend.render(job)?;
        let mut textures: Vec<(Arc<RasterImage>, Arc<RenderImage>)> = Vec::new();
        let mut layers = Vec::new();
        for layer in scene.layers() {
            let pixels = layer.image();
            let image = self
                .textures
                .iter()
                .chain(textures.iter())
                .find(|(old, _)| Arc::ptr_eq(old, pixels))
                .map(|(_, image)| image.clone())
                .map(Ok)
                .unwrap_or_else(|| to_gpui((**pixels).clone()))?;
            textures.push((pixels.clone(), image.clone()));
            layers.push(PositionedLayer {
                image,
                x: layer.x(),
                y: layer.y(),
            });
        }
        self.textures = textures;
        #[cfg(test)]
        {
            self.builds = self.backend.builds();
        }
        let (w, h) = scene.size();
        Ok(Arc::new(LayeredPreview {
            size: size(DevicePixels(w), DevicePixels(h)),
            layers,
            text_regions: scene.text_regions().to_vec(),
            palette_region: scene.palette_region(),
            raster: scene,
            flattened: OnceLock::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::backend::rotation::Rotation;
    use crate::backend::watermark::TextGroup;
    use std::{path::PathBuf, time::SystemTime};

    use super::*;
    use crate::backend::{
        media::ExifInfo,
        watermark::{Placement, TextAlign, TextDirection, WatermarkParams},
    };

    fn job() -> PreviewJob {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/DSC_4587.jpg"
        ));
        let mut group = TextGroup::default();
        group.text.template = vec!["LUMEN FRAME".into()];
        group.text.text_params.truncate(1);
        group.position = Placement::Bottom;
        let mut other = group.clone();
        other.position = Placement::Center;
        other.text.template[0] = "SECOND GROUP".into();
        PreviewJob {
            exif: ExifInfo::read(&path).ok(),
            path,
            params: WatermarkParams::default(),
            text_groups: vec![group, other],
            max_edge: 400,
        }
    }

    fn same(a: &LayeredPreview, b: &LayeredPreview, ix: usize) -> bool {
        Arc::ptr_eq(&a.layers[ix].image, &b.layers[ix].image)
    }

    #[test]
    fn palette_card_shares_text_band_and_reuses_content_when_attached() {
        let mut job = job();
        job.path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/ultra_hdr.jpg"
        )
        .into();
        job.params.palette_card.set_enabled(true);
        job.params.palette_card.set_size(0.2);
        job.params.palette_card.set_show_hex(true);
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        let card = first.layers.last().unwrap();
        let text = &first.layers[3];
        assert!(
            (2 * card.y + card.image.size(0).height.0 - 2 * text.y - text.image.size(0).height.0)
                .abs()
                <= 1,
            "独立色卡与同侧文字应位于同一行"
        );
        assert!(card.x >= 0);
        assert!(card.x + card.image.size(0).width.0 <= first.size.width.0);
        job.params.palette_card.set_position(Placement::Up);
        let moved = cache.render(&job).unwrap();
        assert!(Arc::ptr_eq(
            &card.image,
            &moved.layers.last().unwrap().image
        ));
        for ix in [1, 2, 3, 4] {
            assert!(same(&first, &moved, ix));
        }
        assert!(moved.layers.last().unwrap().y < moved.layers[2].y);
        for side in [
            Placement::Left,
            Placement::Right,
            Placement::Up,
            Placement::Bottom,
        ] {
            job.params.palette_card.set_attachment(Some(
                crate::backend::watermark::GroupAttachment {
                    target: 0,
                    side,
                    gap: 0.02,
                },
            ));
            let attached = cache.render(&job).unwrap();
            let card = attached.layers.last().unwrap();
            assert!(Arc::ptr_eq(
                &card.image,
                &first.layers.last().unwrap().image
            ));
            let (cw, ch) = (card.image.size(0).width.0, card.image.size(0).height.0);
            let region = attached
                .text_regions
                .iter()
                .find(|r| r.group_ix == 0)
                .unwrap();
            let separation = match side {
                Placement::Left => region.x - card.x - cw,
                Placement::Right => card.x - region.x - region.width,
                Placement::Up => region.y - card.y - ch,
                Placement::Bottom => card.y - region.y - region.height,
                _ => unreachable!(),
            };
            let expected_gap =
                (cache.backend.source_height().unwrap() as f64 * 0.02).round() as i32;
            assert!((separation - expected_gap).abs() <= 1);
            assert!(card.x >= 0 && card.y >= 0);
            assert!(card.x + cw <= attached.size.width.0 && card.y + ch <= attached.size.height.0);
            let full = super::super::render_preview(&job).unwrap();
            let max_error = attached
                .image()
                .as_bytes(0)
                .unwrap()
                .iter()
                .zip(full.as_bytes(0).unwrap())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(max_error <= 2, "相邻色卡的预览与导出必须一致");
        }
        job.params.palette_card.set_enabled(false);
        assert_eq!(cache.render(&job).unwrap().layers.len(), 5);
    }

    #[test]
    fn photo_border_matches_full_composition_and_reuses_independent_layers() {
        let mut job = job();
        job.path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/ultra_hdr.jpg"
        )
        .into();
        job.text_groups.clear();
        job.params.solid_background = true;
        job.params.background = [24; 3];
        job.params.shadow_color = [255; 3];
        job.params.border_radius = 0.08;
        job.params.photo_border.set_enabled(true);
        job.params.photo_border.set_width(0.05);
        job.params.photo_border.set_color([240, 40, 90]);
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        assert_eq!(first.layers.len(), 4); // 背景、阴影、边框、照片
        let border = &first.layers[2];
        let photo = &first.layers[3];
        let thickness = (photo.image.size(0).height.0 as f64 * 0.05).round() as i32;
        assert_eq!(
            (border.x, border.y),
            (photo.x - thickness, photo.y - thickness)
        );
        assert_eq!(
            border.image.size(0).width.0,
            photo.image.size(0).width.0 + 2 * thickness
        );
        let size = border.image.size(0);
        let pixels = border.image.as_bytes(0).unwrap();
        let ix = ((size.height.0 / 2 * size.width.0 + thickness / 2) * 4) as usize;
        assert_eq!(&pixels[ix..ix + 4], &[90, 40, 240, 255]);
        assert_eq!(pixels[3], 0, "边框外圆角需要透明");
        let full = super::super::render_preview(&job).unwrap();
        assert_eq!(first.size, full.size(0));
        let error = first
            .image()
            .as_bytes(0)
            .unwrap()
            .iter()
            .zip(full.as_bytes(0).unwrap())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(error <= 2, "边框分层预览与完整合成像素误差 {error}");

        job.params.photo_border.set_color([30, 180, 220]);
        let recolored = cache.render(&job).unwrap();
        for ix in [0, 1, 3] {
            assert!(same(&first, &recolored, ix));
        }
        assert!(!same(&first, &recolored, 2));
        job.params.photo_border.set_width(0.001);
        let thinner = cache.render(&job).unwrap();
        assert!(same(&recolored, &thinner, 3));
        assert!(!same(&recolored, &thinner, 1), "投影必须跟随边框轮廓");
        job.params.photo_border.set_enabled(false);
        assert_eq!(cache.render(&job).unwrap().layers.len(), 3);
    }

    #[test]
    fn editing_one_text_group_keeps_background_shadow_photo_and_other_text_cached() {
        let mut job = job();
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        assert!(first.flattened.get().is_none(), "界面预览不可提前压平");
        let builds = cache.builds;
        job.text_groups[0].text.template[0] = "EDITED GROUP".into();
        let next = cache.render(&job).unwrap();
        assert_eq!(
            cache.builds,
            [builds[0], builds[1], builds[2], builds[3], builds[4] + 1]
        );
        for ix in [0, 1, 2, 4] {
            assert!(same(&first, &next, ix), "未修改图层 {ix} 应保留同一纹理");
        }
        assert!(!same(&first, &next, 3));
        assert_eq!(first.size, next.size);
        assert!(next.flattened.get().is_none());
        // 输出目录与 JPEG 质量不影响任何预览图层。
        job.params.output_folder = Some(PathBuf::from("elsewhere"));
        job.params.quality = 50;
        let builds = cache.builds;
        let unchanged = cache.render(&job).unwrap();
        assert_eq!(cache.builds, builds);
        for ix in 0..first.layers.len() {
            assert!(same(&next, &unchanged, ix));
        }
    }

    #[test]
    fn custom_logo_attachment_changes_only_layer_positions() {
        let mut job = job();
        job.text_groups[1].text.template[0] = "{自定义logo1}".into();
        job.params.custom_logos.insert("自定义logo1".into(), Arc::new(br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="#ffca00"/></svg>"##.to_vec()));
        job.text_groups[1].attachment = Some(crate::backend::watermark::GroupAttachment {
            target: 0,
            side: Placement::Left,
            gap: 0.0,
        });
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        let builds = cache.builds;
        job.text_groups[1].attachment.as_mut().unwrap().gap = 0.03;
        let changed = cache.render(&job).unwrap();
        assert_eq!(cache.builds, builds);
        for ix in 0..first.layers.len() {
            assert!(same(&first, &changed, ix));
        }
        assert_ne!(first.text_regions, changed.text_regions);
    }

    #[test]
    fn custom_text_updates_only_referencing_groups_and_renders_without_exif() {
        let mut job = job();
        job.path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/ultra_hdr.jpg"
        )
        .into();
        job.exif = ExifInfo::read(&job.path).ok();
        job.text_groups[0].border.set_enabled(true);
        job.text_groups[0].border.set_side(Placement::Up, false);
        job.text_groups[0].border.set_side(Placement::Right, false);
        job.text_groups[0].border.set_side(Placement::Bottom, false);
        job.text_groups[0].text.template[0] = "{自定义文本}".into();
        job.params.custom_text = "First title".into();
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        let builds = cache.builds;
        job.params.custom_text = "Second title".into();
        let edited = cache.render(&job).unwrap();
        for ix in [0, 1, 2, 4] {
            assert!(same(&first, &edited, ix));
        }
        assert!(!same(&first, &edited, 3));
        assert_eq!(
            cache.builds,
            [builds[0], builds[1], builds[2], builds[3], builds[4] + 1]
        );
        let full = super::super::render_preview_with_regions(&job).unwrap();
        assert_eq!(edited.text_regions, full.text_regions);
        let error = edited
            .image()
            .as_bytes(0)
            .unwrap()
            .iter()
            .zip(full.image.as_bytes(0).unwrap())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(error <= 2);
        job.exif = None;
        let no_exif = cache.render(&job).unwrap();
        assert_eq!(no_exif.text_regions.len(), 1);
        assert_eq!(no_exif.text_regions[0].group_ix, 0);
        let full = super::super::render_preview_with_regions(&job).unwrap();
        assert_eq!(no_exif.text_regions, full.text_regions);
        job.params.custom_text.clear();
        assert!(cache.render(&job).unwrap().text_regions.is_empty());
    }

    #[test]
    fn gradient_changes_refresh_background_without_rebuilding_unaffected_layers() {
        let mut job = job();
        job.params.gradient_background = true;
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        job.params.gradient = crate::backend::watermark::BackgroundGradient::new(
            270.0,
            job.params.gradient.colors().to_vec(),
        )
        .unwrap();
        let rotated = cache.render(&job).unwrap();
        assert!(!same(&first, &rotated, 0));
        for ix in 1..first.layers.len() {
            assert!(same(&first, &rotated, ix));
        }
        job.params.blur_sigma += 10.0;
        job.params.background = [1, 2, 3];
        let unchanged = cache.render(&job).unwrap();
        for ix in 0..rotated.layers.len() {
            assert!(same(&rotated, &unchanged, ix));
        }
        job.params.gradient = "linear-gradient(270deg,#000,#ff0000,#000)".parse().unwrap();
        let recolored = cache.render(&job).unwrap();
        assert!(!same(&unchanged, &recolored, 0));
        assert!(same(&unchanged, &recolored, 1));
        assert!(same(&unchanged, &recolored, 2));
        job.params.background_grain = true;
        let grain = cache.render(&job).unwrap();
        assert!(!same(&recolored, &grain, 0));
        for ix in 1..grain.layers.len() {
            assert!(same(&recolored, &grain, ix));
        }
        job.params.grain_strength = 0.6;
        let stronger = cache.render(&job).unwrap();
        assert!(!same(&grain, &stronger, 0));
        for ix in 1..stronger.layers.len() {
            assert!(same(&grain, &stronger, ix));
        }
    }

    #[test]
    fn parameter_changes_only_invalidate_their_dependent_layers() {
        let mut job = job();
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        job.params.blur_sigma += 5.0;
        let blurred = cache.render(&job).unwrap();
        assert!(!same(&first, &blurred, 0));
        for ix in 1..first.layers.len() {
            assert!(same(&first, &blurred, ix));
        }
        job.params.shadow_density += 0.5;
        let shadowed = cache.render(&job).unwrap();
        assert!(!same(&blurred, &shadowed, 1));
        for ix in [0, 2, 3, 4] {
            assert!(same(&blurred, &shadowed, ix));
        }
        job.params.shadow_color = [255; 3];
        let white = cache.render(&job).unwrap();
        assert!(!same(&shadowed, &white, 1));
        for ix in [0, 2, 3, 4] {
            assert!(same(&shadowed, &white, ix));
        }
        job.params.border_radius += 0.03;
        let rounded = cache.render(&job).unwrap();
        for ix in [1, 2] {
            assert!(!same(&white, &rounded, ix));
        }
        for ix in [0, 3, 4] {
            assert!(same(&white, &rounded, ix));
        }
        job.text_groups[0].text.text_params[0].size *= 2.0;
        let larger = cache.render(&job).unwrap();
        assert_ne!(rounded.size, larger.size);
        assert!(
            !same(&rounded, &larger, 0),
            "画布变大需要更新 cover 模糊背景"
        );
        for ix in [1, 2, 4] {
            assert!(same(&rounded, &larger, ix));
        }
        job.params.aspect_ratio = Some((1.0, 1.0));
        let larger = cache.render(&job).unwrap();
        job.params.position = Placement::Up;
        let positioned = cache.render(&job).unwrap();
        for ix in 0..larger.layers.len() {
            assert!(same(&larger, &positioned, ix));
        }
        assert_ne!(larger.layers[2].y, positioned.layers[2].y);
    }

    #[test]
    fn rotation_reuses_rendered_layers_and_text_order_keeps_identity() {
        let mut job = job();
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        let builds = cache.builds;
        job.params.rotation = Rotation::Clockwise90;
        let rotated = cache.render(&job).unwrap();
        assert_eq!(cache.builds, builds, "旋转不应重算高斯模糊或排版");
        assert_eq!(rotated.size, size(first.size.height, first.size.width));
        let repeat = cache.render(&job).unwrap();
        for ix in 0..rotated.layers.len() {
            assert!(same(&rotated, &repeat, ix));
        }
        job.text_groups.swap(0, 1);
        let reordered = cache.render(&job).unwrap();
        assert!(Arc::ptr_eq(
            &rotated.layers[3].image,
            &reordered.layers[4].image
        ));
        assert!(Arc::ptr_eq(
            &rotated.layers[4].image,
            &reordered.layers[3].image
        ));
        assert_eq!(cache.builds, builds);
        assert_eq!(reordered.text_regions[0].group_ix, 0);
        assert_eq!(reordered.text_regions[1].group_ix, 1);
        job.text_groups.clear();
        cache.render(&job).unwrap();
        assert!(cache.backend.text_count() == 0, "删除的文字组位图应释放");
    }

    #[test]
    fn source_resolution_and_exif_changes_cannot_reuse_stale_layers() {
        let mut job = job();
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        job.exif.as_mut().unwrap().model = Some("Changed camera".into());
        let exif = cache.render(&job).unwrap();
        for ix in [0, 1, 2] {
            assert!(same(&first, &exif, ix));
        }
        for ix in [3, 4] {
            assert!(!same(&first, &exif, ix));
        }
        job.max_edge = 300;
        let smaller = cache.render(&job).unwrap();
        assert!(smaller.size.width.0 < first.size.width.0);
        for ix in 0..first.layers.len() {
            assert!(!same(&exif, &smaller, ix));
        }
        job.path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/ultra_hdr.jpg"
        ));
        job.exif = ExifInfo::read(&job.path).ok();
        let changed = cache.render(&job).unwrap();
        for ix in 0..changed.layers.len().min(smaller.layers.len()) {
            assert!(!same(&smaller, &changed, ix));
        }
        job.exif = None;
        let missing = cache.render(&job).unwrap();
        assert!(missing.text_regions.is_empty());
        assert_eq!(missing.layers.len(), 3);
    }

    #[test]
    fn replacing_a_photo_at_the_same_path_invalidates_cached_pixels() {
        struct TempPhoto(PathBuf);
        impl Drop for TempPhoto {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let path = TempPhoto(std::env::temp_dir().join(format!(
                "lumen-frame-preview-cache-{}-{}.jpg",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        let mut job = job();
        std::fs::copy(&job.path, &path.0).unwrap();
        job.path = path.0.clone();
        let mut cache = PreviewLayerCache::default();
        let first = cache.render(&job).unwrap();
        let previous = std::fs::metadata(&job.path).unwrap().modified().unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&job.path)
            .unwrap();
        file.set_times(
            std::fs::FileTimes::new().set_modified(previous + std::time::Duration::from_secs(10)),
        )
        .unwrap();
        let next = cache.render(&job).unwrap();
        for ix in 0..first.layers.len() {
            assert!(!same(&first, &next, ix));
        }
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/ultra_hdr.jpg"
            ),
            &job.path,
        )
        .unwrap();
        let replaced = cache.render(&job).unwrap();
        assert_ne!(next.size, replaced.size);
        assert!(!same(&next, &replaced, 0));
    }

    #[test]
    fn inside_anchors_keep_canvas_size_and_match_full_composition() {
        use crate::backend::watermark::{PhotoAnchor, PhotoPlacement};
        let mut job = job();
        job.text_groups.truncate(1);
        job.text_groups[0].text.template = vec!["Inside".into()];
        job.text_groups[0].text.text_params.truncate(1);
        let mut empty = job.clone();
        empty.text_groups.clear();
        let expected_size = PreviewLayerCache::default().render(&empty).unwrap().size();
        for anchor in [
            PhotoAnchor::TopLeft,
            PhotoAnchor::Top,
            PhotoAnchor::TopRight,
            PhotoAnchor::Left,
            PhotoAnchor::Right,
            PhotoAnchor::BottomLeft,
            PhotoAnchor::Bottom,
            PhotoAnchor::BottomRight,
        ] {
            job.text_groups[0].photo_placement = Some(PhotoPlacement::new(anchor));
            let scene = PreviewLayerCache::default().render(&job).unwrap();
            assert_eq!(scene.size(), expected_size);
            let photo = scene.photo_region();
            let region = scene.text_regions[0];
            assert!(region.x >= photo.x && region.y >= photo.y);
            assert!(region.x + region.width <= photo.x + photo.width);
            assert!(region.y + region.height <= photo.y + photo.height);
            let x = (region.x + region.width / 2 - photo.x) as f64 / photo.width as f64;
            let y = (region.y + region.height / 2 - photo.y) as f64 / photo.height as f64;
            assert_eq!(
                PhotoAnchor::from_normalized_point(x, y, Rotation::None),
                anchor
            );
        }
        // 超长文字的裁切、竖排和旋转也复用同一规则，并保留 HDR 预览管线。
        job.path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/ultra_hdr.jpg"
        )
        .into();
        job.exif = Some(ExifInfo::default());
        job.text_groups[0].text.template[0] = "{自定义文本}".into();
        job.params.custom_text = "Long text ".repeat(80);
        job.text_groups[0].direction = TextDirection::Vertical;
        job.params.rotation = Rotation::Clockwise90;
        let scene = PreviewLayerCache::default().render(&job).unwrap();
        let full = super::super::render_preview_with_regions(&job).unwrap();
        assert_eq!(scene.text_regions, full.text_regions);
        let photo = scene.photo_region();
        let region = scene.text_regions[0];
        assert!(region.x >= photo.x && region.y >= photo.y);
        assert!(region.x + region.width <= photo.x + photo.width);
        assert!(region.y + region.height <= photo.y + photo.height);
        let max_error = scene
            .image()
            .as_bytes(0)
            .unwrap()
            .iter()
            .zip(full.image.as_bytes(0).unwrap())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(max_error <= 2, "内部文字的预览与导出像素差异：{max_error}");
    }

    #[test]
    fn layered_pixels_and_click_regions_match_full_composition_including_hdr_and_rotation() {
        for path in [
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/DSC_4587.jpg"
            ),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/ultra_hdr.jpg"
            ),
        ] {
            for rotation in [
                Rotation::None,
                Rotation::Clockwise90,
                Rotation::HalfTurn,
                Rotation::CounterClockwise90,
            ] {
                for kind in [
                    crate::backend::watermark::BackgroundKind::Photo,
                    crate::backend::watermark::BackgroundKind::Solid,
                    crate::backend::watermark::BackgroundKind::Gradient,
                ] {
                    let mut job = job();
                    job.path = path.into();
                    job.exif = ExifInfo::read(&job.path).ok();
                    job.params.rotation = rotation;
                    job.params.palette_card.set_enabled(true);
                    job.params.palette_card.set_show_hex(true);
                    job.params.palette_card.set_show_ratio(true);
                    job.params
                        .palette_card
                        .set_shape(crate::backend::watermark::PaletteShape::Rounded);
                    job.params.palette_card.set_proportional(true);
                    job.params.palette_card.set_gap(0.0);
                    job.params.palette_card.set_selected(vec![0, 2, 4]);
                    job.params.set_background_kind(kind);
                    job.max_edge = 240;
                    job.text_groups[0].text.template[0] = "{Logo} {型号}".into();
                    job.text_groups[0].position = Placement::Right;
                    job.text_groups[0].direction = TextDirection::Vertical;
                    job.text_groups[0].align = TextAlign::Right;
                    job.text_groups[0].padding = 0.08;
                    let scene = PreviewLayerCache::default().render(&job).unwrap();
                    let full = super::super::render_preview_with_regions(&job).unwrap();
                    assert_eq!(scene.size, full.image.size(0));
                    assert_eq!(scene.text_regions, full.text_regions);
                    let max_error = scene
                        .image()
                        .as_bytes(0)
                        .unwrap()
                        .iter()
                        .zip(full.image.as_bytes(0).unwrap())
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    assert!(
                        max_error <= 2,
                        "{path} {rotation:?} background={kind:?}: 像素误差 {max_error}"
                    );
                }
            }
        }
    }

    #[test]
    fn sixteen_bit_photo_layer_matches_export_colour_conversion() {
        let path = std::env::temp_dir().join(format!(
            "lumen-frame-16-bit-preview-{}.png",
            std::process::id()
        ));
        let pixels = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_fn(80, 60, |x, y| {
            image::Rgb([(x * 700) as u16, (y * 900) as u16, 30000])
        });
        pixels.save(&path).unwrap();
        let mut job = job();
        job.path = path.clone();
        job.exif = None;
        job.params.solid_background = true;
        job.params.shadow_size = 0.0;
        job.params.border_radius = 0.0;
        let scene = PreviewLayerCache::default().render(&job).unwrap();
        let full = super::super::render_preview(&job).unwrap();
        let _ = std::fs::remove_file(path);
        let error = scene
            .image()
            .as_bytes(0)
            .unwrap()
            .iter()
            .zip(full.as_bytes(0).unwrap())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(error <= 2, "16-bit 转换像素误差 {error}");
    }

    #[test]
    #[ignore = "manual preview performance measurement"]
    fn measure_text_edit_preview_speed() {
        let mut job = job();
        job.max_edge = 1600;
        job.params.blur_sigma = 40.0;
        let mut cache = PreviewLayerCache::default();
        cache.render(&job).unwrap();
        let start = std::time::Instant::now();
        for ix in 0..10 {
            job.text_groups[0].text.template[0] = format!("TEXT EDIT {ix:02}");
            super::super::render_preview(&job).unwrap();
        }
        let full = start.elapsed();
        let start = std::time::Instant::now();
        for ix in 0..10 {
            job.text_groups[0].text.template[0] = format!("TEXT EDIT {ix:02}");
            cache.render(&job).unwrap();
        }
        eprintln!(
            "10 text edits: full={full:?}, layered={:?}, builds={:?}",
            start.elapsed(),
            cache.builds
        );
    }
}
