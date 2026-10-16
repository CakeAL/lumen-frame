//! 只缓存当前照片的已求值位图；不把非 Send 的 libvips 句柄带出后台线程。

use super::{PreviewJob, RasterImage, load_scaled, rasterize};
use crate::{
    media::{ensure_vips, vips::VipsImage},
    photo::{
        Photo,
        layers::{LayerRenderer, PositionedLayer},
    },
    render::{canvas, image as photo_image, text::TextGroupRegion},
    rotation::Rotation,
    watermark::{BackgroundGradient, PaletteCard, TextGroup},
};
use anyhow::{Context as _, Result};
use image::{ImageBuffer, Rgba};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::SystemTime,
};

/// 分层预览快照。界面直接绘制各层，只有显式读取完整位图时才在 CPU 上压平。
pub struct LayeredPreview {
    size: (i32, i32),
    layers: Vec<PreviewLayer>,
    text_regions: Vec<TextGroupRegion>,
    palette_region: Option<TextGroupRegion>,
    flattened: OnceLock<Arc<RasterImage>>,
}

impl LayeredPreview {
    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    pub fn image(&self) -> &Arc<RasterImage> {
        self.flattened.get_or_init(|| {
            let (w, h) = (self.size.0 as u32, self.size.1 as u32);
            let mut pixels = vec![0u8; w as usize * h as usize * 4];
            for layer in &self.layers {
                let size = (layer.image.width() as i32, layer.image.height() as i32);
                let bytes = layer.image.pixels();
                for y in layer.y.max(0)..(layer.y + size.1).min(h as i32) {
                    for x in layer.x.max(0)..(layer.x + size.0).min(w as i32) {
                        let src_ix = (((y - layer.y) * size.0 + x - layer.x) * 4) as usize;
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
    custom_text: String,
    custom_logos: Vec<(String, Option<Arc<Vec<u8>>>)>,
}

#[derive(Clone)]
struct TextLayer {
    image: Arc<RasterImage>,
    visible: Option<TextGroupRegion>,
}

#[derive(Clone, PartialEq)]
struct BackgroundKey {
    size: (i32, i32),
    solid: bool,
    color: [u8; 3],
    blur: f64,
    gradient: Option<BackgroundGradient>,
}

#[derive(PartialEq)]
struct ShadowKey {
    radius: f64,
    size: f64,
    density: f64,
    color: [u8; 3],
    border: i32,
}

#[derive(PartialEq)]
struct BorderKey {
    radius: f64,
    width: i32,
    color: [u8; 3],
}

#[derive(Clone)]
struct ShadowImage {
    image: Arc<RasterImage>,
    x: i32,
    y: i32,
}

struct CachedShadow {
    key: ShadowKey,
    image: Option<ShadowImage>,
}

#[derive(Default)]
pub struct PreviewLayerCache {
    source: Option<(SourceKey, (i32, i32))>,
    background: Option<(BackgroundKey, Arc<RasterImage>)>,
    photo: Option<(f64, Arc<RasterImage>)>,
    shadow: Option<CachedShadow>,
    border: Option<(BorderKey, ShadowImage)>,
    texts: Vec<(TextKey, Option<TextLayer>)>,
    palette: Option<((PaletteCard, String), Option<TextLayer>)>,
    rotated: Vec<(Arc<RasterImage>, Rotation, Arc<RasterImage>)>,
    #[cfg(feature = "test-support")]
    pub(crate) builds: [usize; 5], // 解码、背景、照片、阴影、文字
}

impl PreviewLayerCache {
    #[cfg(feature = "test-support")]
    pub fn is_empty(&self) -> bool {
        self.source.is_none() && self.texts.is_empty() && self.rotated.is_empty()
    }

    pub fn render(&mut self, job: &PreviewJob) -> Result<Arc<LayeredPreview>> {
        ensure_vips();
        let resolved_job = PreviewJob {
            params: crate::photo_palette::resolve(&job.path, &job.params)?,
            ..job.clone()
        };
        let job = &resolved_job;
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
            #[cfg(feature = "test-support")]
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
        let mut palette_region = None;
        let mut next_rotated = Vec::new();
        for (layer_ix, layer) in scene.layers.into_iter().enumerate() {
            if Some(layer_ix) == scene.palette_layer_ix {
                palette_region = layer.image.visible.and_then(|region| {
                    region.positioned(0, (layer.x, layer.y), scene.size, job.params.rotation)
                });
            }
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
            let size = (image.width() as i32, image.height() as i32);
            let rect = TextGroupRegion {
                group_ix: 0,
                x: layer.x,
                y: layer.y,
                width: size.0,
                height: size.1,
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
            layers.push(PreviewLayer {
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
            size: (w, h),
            layers,
            text_regions: regions,
            palette_region,
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
            #[cfg(feature = "test-support")]
            {
                self.cache.builds[0] += 1;
            }
        }
        Ok(self.base.as_ref().unwrap())
    }

    fn rounded(&mut self) -> Result<VipsImage> {
        let radius = self.job.params.border_radius;
        if let Some((_, image)) = self.cache.photo.as_ref().filter(|(old, _)| *old == radius) {
            let size = (image.width() as i32, image.height() as i32);
            let pixels = image.pixels().to_vec();
            let image = VipsImage::from_memory(
                pixels,
                size.0 as u32,
                size.1 as u32,
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
        let size = (image.image.width() as i32, image.image.height() as i32);
        (size.0, size.1)
    }

    fn content_bounds(image: &TextLayer) -> Result<(i32, i32, i32, i32)> {
        let (w, h) = Self::dimensions(image);
        Ok(image
            .visible
            .map(|region| (region.x, region.y, region.width, region.height))
            .unwrap_or((0, 0, w, h)))
    }

    fn text(&mut self, group: &TextGroup) -> Result<Option<TextLayer>> {
        let key = TextKey {
            // 相邻排版只改变位置，不应重绘内容纹理。
            group: TextGroup {
                attachment: None,
                ..group.clone()
            },
            default_font: self.job.params.default_font.clone(),
            background: self
                .job
                .params
                .text_background_color()
                .unwrap_or(self.job.params.background),
            solid: self.job.params.text_background_color().is_some(),
            exif: format!("{:?}", self.job.exif),
            custom_logos: group
                .custom_logo_names()
                .into_iter()
                .map(|name| {
                    let bytes = self.job.params.custom_logos.get(&name).cloned();
                    (name, bytes)
                })
                .collect(),
            custom_text: if group.uses_custom_text() {
                self.job.params.custom_text.clone()
            } else {
                String::new()
            },
        };
        let image = if let Some((_, image)) = self.cache.texts.iter().find(|(old, _)| *old == key) {
            image.clone()
        } else {
            #[cfg(feature = "test-support")]
            {
                self.cache.builds[4] += 1;
            }
            group
                .render_for_photo(self.job.exif.as_ref(), self.dimensions.1, &self.job.params)?
                .map(|image| {
                    let visible = TextGroupRegion::from_layer(
                        &image,
                        0,
                        (0, 0),
                        (image.width() as i32, image.height() as i32),
                        Rotation::None,
                    )?;
                    Ok::<_, anyhow::Error>(TextLayer {
                        image: raster_image(&image)?,
                        visible,
                    })
                })
                .transpose()?
        };
        self.next_texts.push((key, image.clone()));
        Ok(image)
    }

    fn palette(&mut self) -> Result<Option<TextLayer>> {
        let params = &self.job.params;
        let mut content = params.palette_card.clone();
        // 放置参数只移动纹理，不重新栅格化 SVG 和色号。
        content.set_position(crate::watermark::Placement::Center);
        content.set_align(crate::watermark::TextAlign::Center);
        content.set_padding(0.0);
        content.set_attachment(None);
        let key = (content, params.default_font.clone());
        if let Some((_, image)) = self.cache.palette.as_ref().filter(|(old, _)| *old == key) {
            return Ok(image.clone());
        }
        let image = crate::render::palette_card::render(self.dimensions, params)?
            .map(|image| {
                let visible = TextGroupRegion::from_layer(
                    &image,
                    0,
                    (0, 0),
                    (image.width() as i32, image.height() as i32),
                    Rotation::None,
                )?;
                Ok::<_, anyhow::Error>(TextLayer {
                    image: raster_image(&image)?,
                    visible,
                })
            })
            .transpose()?;
        self.cache.palette = Some((key, image.clone()));
        Ok(image)
    }

    fn background(&mut self, size: (i32, i32)) -> Result<TextLayer> {
        let params = &self.job.params;
        let key = BackgroundKey {
            size,
            solid: !params.gradient_background && params.solid_background,
            color: if !params.gradient_background && params.solid_background {
                params.background
            } else {
                [0; 3]
            },
            blur: if params.solid_background || params.gradient_background {
                0.0
            } else {
                params.blur_sigma
            },
            gradient: params.gradient_background.then(|| params.gradient.clone()),
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
            let image = raster_image(&image)?;
            self.cache.background = Some((key, image.clone()));
            #[cfg(feature = "test-support")]
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
                let image = raster_image(&self.rounded()?)?;
                self.cache.photo = Some((radius, image.clone()));
                #[cfg(feature = "test-support")]
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
            color: p.shadow_color,
            border: p.photo_border.pixels(self.dimensions.1),
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
                canvas::photo_shadow_layer(&self.rounded()?, &self.job.params)?
                    .map(|(image, x, y)| {
                        Ok::<_, anyhow::Error>(ShadowImage {
                            image: raster_image(&image)?,
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
            #[cfg(feature = "test-support")]
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

    fn border(&mut self) -> Result<Option<PositionedLayer<TextLayer>>> {
        let p = &self.job.params;
        let key = BorderKey {
            radius: p.border_radius,
            width: p.photo_border.pixels(self.dimensions.1),
            color: p.photo_border.color(),
        };
        if key.width == 0 {
            self.cache.border = None;
            return Ok(None);
        }
        let image = if let Some((_, image)) =
            self.cache.border.as_ref().filter(|(old, _)| *old == key)
        {
            image.clone()
        } else {
            let (image, x, y) = photo_image::border_layer(self.dimensions.0, self.dimensions.1, p)?
                .expect("启用的照片边框");
            let image = ShadowImage {
                image: raster_image(&image)?,
                x,
                y,
            };
            self.cache.border = Some((key, image.clone()));
            image
        };
        Ok(Some(PositionedLayer {
            image: TextLayer {
                image: image.image,
                visible: None,
            },
            x: image.x,
            y: image.y,
            group_ix: None,
        }))
    }
}

fn raster_image(image: &VipsImage) -> Result<Arc<RasterImage>> {
    rasterize(image).map(Arc::new)
}
fn bitmap(width: u32, height: u32, pixels: Vec<u8>) -> Arc<RasterImage> {
    Arc::new(RasterImage {
        width,
        height,
        pixels,
    })
}
fn rotate_bitmap(image: &RasterImage, rotation: Rotation) -> Arc<RasterImage> {
    let buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(
        image.width(),
        image.height(),
        image.pixels().to_vec(),
    )
    .unwrap();
    let buffer = match rotation {
        Rotation::Clockwise90 => image::imageops::rotate90(&buffer),
        Rotation::HalfTurn => image::imageops::rotate180(&buffer),
        Rotation::CounterClockwise90 => image::imageops::rotate270(&buffer),
        Rotation::None => buffer,
    };
    bitmap(buffer.width(), buffer.height(), buffer.into_raw())
}

/// 已求值的 RGBA 图层；坐标属于最终输出画布。
pub struct PreviewLayer {
    image: Arc<RasterImage>,
    x: i32,
    y: i32,
    group_ix: Option<usize>,
}
impl PreviewLayer {
    pub fn image(&self) -> &Arc<RasterImage> {
        &self.image
    }
    pub fn x(&self) -> i32 {
        self.x
    }
    pub fn y(&self) -> i32 {
        self.y
    }
    pub fn group_ix(&self) -> Option<usize> {
        self.group_ix
    }
}
impl LayeredPreview {
    pub fn layers(&self) -> &[PreviewLayer] {
        &self.layers
    }
    pub fn text_regions(&self) -> &[TextGroupRegion] {
        &self.text_regions
    }
    pub fn palette_region(&self) -> Option<TextGroupRegion> {
        self.palette_region
    }
}
#[cfg(feature = "test-support")]
impl PreviewLayerCache {
    pub fn builds(&self) -> [usize; 5] {
        self.builds
    }
    pub fn source_height(&self) -> Option<i32> {
        self.source.as_ref().map(|(_, size)| size.1)
    }
    pub fn text_count(&self) -> usize {
        self.texts.len()
    }
}
