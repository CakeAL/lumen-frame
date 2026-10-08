//! 只缓存当前照片的已求值位图；不把非 Send 的 libvips 句柄带出后台线程。

use super::{PreviewJob, load_scaled, to_render_image};
use crate::{
    media::{ensure_vips, vips::VipsImage},
    photo::{
        Photo,
        layers::{LayerRenderer, PositionedLayer},
    },
    render::{canvas, image as photo_image, text::TextGroupRegion},
    rotation::Rotation,
    watermark::TextGroup,
};
use anyhow::{Context as _, Result};
use gpui_kit::{DevicePixels, RenderImage, Size, size};
use image::{Frame, ImageBuffer, Rgba};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::SystemTime,
};

/// 分层预览快照。界面直接绘制各层，只有显式读取完整位图时才在 CPU 上压平。
pub struct LayeredPreview {
    size: Size<DevicePixels>,
    pub(crate) layers: Vec<PositionedLayer<Arc<RenderImage>>>,
    pub(crate) text_regions: Vec<TextGroupRegion>,
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

    pub(crate) fn image(&self) -> &Arc<RenderImage> {
        self.flattened.get_or_init(|| {
            let (w, h) = (self.size.width.0 as u32, self.size.height.0 as u32);
            let mut pixels = vec![0u8; w as usize * h as usize * 4];
            for layer in &self.layers {
                let size = layer.image.size(0);
                let bytes = layer.image.as_bytes(0).expect("缓存图层像素");
                for y in layer.y.max(0)..(layer.y + size.height.0).min(h as i32) {
                    for x in layer.x.max(0)..(layer.x + size.width.0).min(w as i32) {
                        let src_ix = (((y - layer.y) * size.width.0 + x - layer.x) * 4) as usize;
                        let dst_ix = ((y as u32 * w + x as u32) * 4) as usize;
                        let src = &bytes[src_ix..src_ix + 4];
                        let dst = &mut pixels[dst_ix..dst_ix + 4];
                        let alpha = src[3] as f32 / 255.0;
                        let old_alpha = dst[3] as f32 / 255.0;
                        let total = alpha + old_alpha * (1.0 - alpha);
                        if total > 0.0 {
                            for c in 0..3 {
                                dst[c] = ((src[c] as f32 * alpha
                                    + dst[c] as f32 * old_alpha * (1.0 - alpha))
                                    / total) as u8;
                            }
                            dst[3] = (total * 255.0).round() as u8;
                        }
                    }
                }
            }
            bitmap(w, h, pixels)
        })
    }
}

#[derive(PartialEq)]
struct SourceKey {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    max_edge: i32,
}

#[derive(Clone, PartialEq)]
struct TextKey {
    group: TextGroup,
    default_font: String,
    background: [u8; 3],
    solid: bool,
    exif: String,
}

#[derive(Clone)]
struct TextLayer {
    image: Arc<RenderImage>,
    visible: Option<TextGroupRegion>,
}

#[derive(Clone, PartialEq)]
struct BackgroundKey {
    size: (i32, i32),
    solid: bool,
    color: [u8; 3],
    blur: f64,
}

#[derive(PartialEq)]
struct ShadowKey {
    radius: f64,
    size: f64,
    density: f64,
}

#[derive(Clone)]
struct ShadowImage {
    image: Arc<RenderImage>,
    x: i32,
    y: i32,
}

struct CachedShadow {
    key: ShadowKey,
    image: Option<ShadowImage>,
}

#[derive(Default)]
pub(crate) struct PreviewLayerCache {
    source: Option<(SourceKey, (i32, i32))>,
    background: Option<(BackgroundKey, Arc<RenderImage>)>,
    photo: Option<(f64, Arc<RenderImage>)>,
    shadow: Option<CachedShadow>,
    texts: Vec<(TextKey, Option<TextLayer>)>,
    rotated: Vec<(Arc<RenderImage>, Rotation, Arc<RenderImage>)>,
    #[cfg(test)]
    pub(crate) builds: [usize; 5], // 解码、背景、照片、阴影、文字
}

impl PreviewLayerCache {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.source.is_none() && self.texts.is_empty() && self.rotated.is_empty()
    }

    pub(crate) fn render(&mut self, job: &PreviewJob) -> Result<Arc<LayeredPreview>> {
        ensure_vips();
        let metadata = std::fs::metadata(&job.path).context("读取预览照片信息")?;
        let key = SourceKey {
            path: job.path.clone(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
            max_edge: job.max_edge,
        };
        let mut base = None;
        if self.source.as_ref().is_none_or(|(old, _)| *old != key) {
            *self = Self::default();
            let image = load_scaled(&job.path, job.max_edge).context("读取预览底图")?;
            self.source = Some((key, (image.width() as i32, image.height() as i32)));
            base = Some(image);
            #[cfg(test)]
            {
                self.builds[0] += 1;
            }
        }
        let dimensions = self.source.as_ref().unwrap().1;
        let mut renderer = CachedRenderer {
            job,
            cache: self,
            base,
            dimensions,
            next_texts: Vec::new(),
        };
        let scene = Photo::prepare_watermark_layers(
            &mut renderer,
            dimensions,
            &job.params,
            &job.text_groups,
        )?;
        renderer.cache.texts = renderer.next_texts;
        let mut layers = Vec::with_capacity(scene.layers.len());
        let mut regions = Vec::new();
        let mut next_rotated = Vec::new();
        for layer in scene.layers {
            if let Some(group_ix) = layer.group_ix
                && let Some(region) = layer.image.visible
                && let Some(region) = region.positioned(
                    group_ix,
                    (layer.x, layer.y),
                    scene.size,
                    job.params.rotation,
                )
            {
                regions.push(region);
            }
            let image = &layer.image.image;
            let size = image.size(0);
            let rect = TextGroupRegion {
                group_ix: 0,
                x: layer.x,
                y: layer.y,
                width: size.width.0,
                height: size.height.0,
            }
            .rotated(job.params.rotation, scene.size);
            let rotated = if job.params.rotation == Rotation::None {
                image.clone()
            } else {
                let result = self
                    .rotated
                    .iter()
                    .find(|(original, rotation, _)| {
                        Arc::ptr_eq(original, image) && *rotation == job.params.rotation
                    })
                    .map(|(_, _, image)| image.clone())
                    .unwrap_or_else(|| rotate_bitmap(image, job.params.rotation));
                next_rotated.push((image.clone(), job.params.rotation, result.clone()));
                result
            };
            layers.push(PositionedLayer {
                image: rotated,
                x: rect.x,
                y: rect.y,
                group_ix: layer.group_ix,
            });
        }
        self.rotated = next_rotated;
        let (w, h) = scene.size;
        let (w, h) = match job.params.rotation {
            Rotation::Clockwise90 | Rotation::CounterClockwise90 => (h, w),
            _ => (w, h),
        };
        Ok(Arc::new(LayeredPreview {
            size: size(DevicePixels(w), DevicePixels(h)),
            layers,
            text_regions: regions,
            flattened: OnceLock::new(),
        }))
    }
}

struct CachedRenderer<'a> {
    job: &'a PreviewJob,
    cache: &'a mut PreviewLayerCache,
    base: Option<VipsImage>,
    dimensions: (i32, i32),
    next_texts: Vec<(TextKey, Option<TextLayer>)>,
}

impl CachedRenderer<'_> {
    fn base(&mut self) -> Result<&VipsImage> {
        if self.base.is_none() {
            self.base =
                Some(load_scaled(&self.job.path, self.job.max_edge).context("读取预览底图")?);
            #[cfg(test)]
            {
                self.cache.builds[0] += 1;
            }
        }
        Ok(self.base.as_ref().unwrap())
    }

    fn rounded(&mut self) -> Result<VipsImage> {
        let radius = self.job.params.border_radius;
        if let Some((_, image)) = self.cache.photo.as_ref().filter(|(old, _)| *old == radius) {
            let size = image.size(0);
            let mut pixels = image.as_bytes(0).unwrap().to_vec();
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            let image = VipsImage::from_memory(
                pixels,
                size.width.0 as u32,
                size.height.0 as u32,
                4,
                vips::VipsBandFormat::VIPS_FORMAT_UCHAR,
            )?;
            // memory 图像默认是 MULTIBAND；声明 sRGB 才能让 hasalpha 正确识别圆角覆盖率。
            return Ok(crate::media::vips::image_op(|out| unsafe {
                vips_sys::vips_copy(
                    image.as_ptr(),
                    out,
                    c"interpretation".as_ptr(),
                    vips::VipsInterpretation::VIPS_INTERPRETATION_sRGB,
                    std::ptr::null::<i8>(),
                )
            })?);
        }
        let base = self.base()?;
        let image = crate::media::vips::image_op(|out| unsafe {
            vips_sys::vips_copy(base.as_ptr(), out, std::ptr::null::<i8>())
        })?;
        Ok(if radius > 0.0 {
            photo_image::add_round_corner(image, radius)?
        } else {
            image
        })
    }
}

impl LayerRenderer for CachedRenderer<'_> {
    type Image = TextLayer;
    fn dimensions(image: &TextLayer) -> (i32, i32) {
        let size = image.image.size(0);
        (size.width.0, size.height.0)
    }

    fn text(&mut self, group: &TextGroup) -> Result<Option<TextLayer>> {
        let key = TextKey {
            group: group.clone(),
            default_font: self.job.params.default_font.clone(),
            background: self.job.params.background,
            solid: self.job.params.solid_background,
            exif: format!("{:?}", self.job.exif),
        };
        let image = if let Some((_, image)) = self.cache.texts.iter().find(|(old, _)| *old == key) {
            image.clone()
        } else if let Some(exif) = &self.job.exif {
            #[cfg(test)]
            {
                self.cache.builds[4] += 1;
            }
            group
                .render_text(exif, self.dimensions.1, &self.job.params)?
                .map(|image| {
                    let visible = TextGroupRegion::from_layer(
                        &image,
                        0,
                        (0, 0),
                        (image.width() as i32, image.height() as i32),
                        Rotation::None,
                    )?;
                    Ok::<_, anyhow::Error>(TextLayer {
                        image: to_render_image(&image)?,
                        visible,
                    })
                })
                .transpose()?
        } else {
            None
        };
        self.next_texts.push((key, image.clone()));
        Ok(image)
    }

    fn background(&mut self, size: (i32, i32)) -> Result<TextLayer> {
        let params = &self.job.params;
        let key = BackgroundKey {
            size,
            solid: params.solid_background,
            color: if params.solid_background {
                params.background
            } else {
                [0; 3]
            },
            blur: if params.solid_background {
                0.0
            } else {
                params.blur_sigma
            },
        };
        let image = if let Some((_, image)) = self
            .cache
            .background
            .as_ref()
            .filter(|(old, _)| *old == key)
        {
            image.clone()
        } else {
            let params = self.job.params.clone();
            let image = canvas::new_canvas(size.0, size.1, self.base()?, &params)?;
            let image = to_render_image(&image)?;
            self.cache.background = Some((key, image.clone()));
            #[cfg(test)]
            {
                self.cache.builds[1] += 1;
            }
            image
        };
        Ok(TextLayer {
            image,
            visible: None,
        })
    }

    fn photo(&mut self) -> Result<TextLayer> {
        let radius = self.job.params.border_radius;
        let image =
            if let Some((_, image)) = self.cache.photo.as_ref().filter(|(old, _)| *old == radius) {
                image.clone()
            } else {
                let image = to_render_image(&self.rounded()?)?;
                self.cache.photo = Some((radius, image.clone()));
                #[cfg(test)]
                {
                    self.cache.builds[2] += 1;
                }
                image
            };
        Ok(TextLayer {
            image,
            visible: None,
        })
    }

    fn shadow(&mut self) -> Result<Option<PositionedLayer<TextLayer>>> {
        let p = &self.job.params;
        let key = ShadowKey {
            radius: p.border_radius,
            size: p.shadow_size,
            density: p.shadow_density,
        };
        let image = if let Some(image) = self
            .cache
            .shadow
            .as_ref()
            .filter(|cached| cached.key == key)
        {
            image.image.clone()
        } else {
            let image = if p.shadow_size <= 0.0 || p.shadow_density <= 0.0 {
                None
            } else {
                canvas::shadow_layer(&self.rounded()?, &self.job.params)?
                    .map(|(image, x, y)| {
                        Ok::<_, anyhow::Error>(ShadowImage {
                            image: to_render_image(&image)?,
                            x,
                            y,
                        })
                    })
                    .transpose()?
            };
            self.cache.shadow = Some(CachedShadow {
                key,
                image: image.clone(),
            });
            #[cfg(test)]
            {
                self.cache.builds[3] += 1;
            }
            image
        };
        Ok(image.map(|ShadowImage { image, x, y }| PositionedLayer {
            image: TextLayer {
                image,
                visible: None,
            },
            x,
            y,
            group_ix: None,
        }))
    }
}

fn bitmap(w: u32, h: u32, pixels: Vec<u8>) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![Frame::new(
        ImageBuffer::<Rgba<u8>, _>::from_raw(w, h, pixels).unwrap(),
    )]))
}

fn rotate_bitmap(image: &RenderImage, rotation: Rotation) -> Arc<RenderImage> {
    let size = image.size(0);
    let buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(
        size.width.0 as u32,
        size.height.0 as u32,
        image.as_bytes(0).unwrap().to_vec(),
    )
    .unwrap();
    let buffer = match rotation {
        Rotation::Clockwise90 => image::imageops::rotate90(&buffer),
        Rotation::HalfTurn => image::imageops::rotate180(&buffer),
        Rotation::CounterClockwise90 => image::imageops::rotate270(&buffer),
        Rotation::None => buffer,
    };
    Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        media::ExifInfo,
        watermark::{Placement, TextAlign, TextDirection, WatermarkParams},
    };

    fn job() -> PreviewJob {
        let path = PathBuf::from("./test_images/DSC_4587.jpg");
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
        job.params.border_radius += 0.03;
        let rounded = cache.render(&job).unwrap();
        for ix in [1, 2] {
            assert!(!same(&shadowed, &rounded, ix));
        }
        for ix in [0, 3, 4] {
            assert!(same(&shadowed, &rounded, ix));
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
        assert!(cache.texts.is_empty(), "删除的文字组位图应释放");
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
        job.path = PathBuf::from("./test_images/ultra_hdr.jpg");
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
        std::fs::copy("./test_images/ultra_hdr.jpg", &job.path).unwrap();
        let replaced = cache.render(&job).unwrap();
        assert_ne!(next.size, replaced.size);
        assert!(!same(&next, &replaced, 0));
    }

    #[test]
    fn layered_pixels_and_click_regions_match_full_composition_including_hdr_and_rotation() {
        for path in ["./test_images/DSC_4587.jpg", "./test_images/ultra_hdr.jpg"] {
            for rotation in [
                Rotation::None,
                Rotation::Clockwise90,
                Rotation::HalfTurn,
                Rotation::CounterClockwise90,
            ] {
                for solid in [false, true] {
                    let mut job = job();
                    job.path = path.into();
                    job.exif = ExifInfo::read(&job.path).ok();
                    job.params.rotation = rotation;
                    job.params.solid_background = solid;
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
                        "{path} {rotation:?} solid={solid}: 像素误差 {max_error}"
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
