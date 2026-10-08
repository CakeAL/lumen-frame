//! 预览与缩略图的位图生成。
//!
//! [`Photo::compose_watermark`] 与分层预览共用照片用例中的排版管线。
//! 这一层负责缩小底图、转换 GPUI 位图，以及缓存可独立更新的预览图层。
//!
//! 这里的函数都会阻塞，只能在后台线程里调用。

mod layers;

pub use layers::LayeredPreview;
pub(crate) use layers::PreviewLayerCache;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use gpui_kit::RenderImage;
use image::{Frame, ImageBuffer, Rgba};
use vips::{VipsBandFormat, VipsSize};
use vips_sys::VipsForeignKeep;

use crate::{
    features::colour_gainmap,
    gainmap as gain_map,
    media::{
        ExifInfo, ensure_vips, image_write_to_memory, load_base_image,
        vips::{VipsImage, from_owned_ptr, image_op},
    },
    photo::Photo,
    rotation::Rotation,
    watermark::{TextGroup, WatermarkParams},
};

/// 100% 预览底图的长边上限（px）。
///
/// 预览要的是看清效果，不是像素级还原：底图缩到这个尺寸后，合成耗时基本与原图大小
/// 无关，拖动滑块才跟得上手。
pub const PREVIEW_DEFAULT_MAX_EDGE: i32 = 1600;

/// 队列缩略图的长边上限（px）。
pub const THUMBNAIL_MAX_EDGE: i32 = 320;

/// 一次预览渲染所需的全部输入。
///
/// 它是一份自包含快照：后台线程拿到的值不会在渲染过程中被用户的下一次拖动改写，
/// 所以预览永远对应界面上某一个确定的状态。
#[derive(Clone)]
pub struct PreviewJob {
    pub path: PathBuf,
    pub exif: Option<ExifInfo>,
    pub params: WatermarkParams,
    pub text_groups: Vec<TextGroup>,
    /// 本次渲染的预览底图长边上限；由设置页在请求时固定下来。
    pub max_edge: i32,
}

/// 按预览分辨率渲染水印照片。会阻塞，请在后台线程调用。
pub fn render_preview(job: &PreviewJob) -> Result<Arc<RenderImage>> {
    ensure_vips();
    let base = load_scaled(&job.path, job.max_edge).context("读取预览底图")?;
    let composed = Photo::compose_watermark(base, job.exif.as_ref(), &job.params, &job.text_groups)
        .context("合成预览")?;
    to_render_image(&composed).context("转换预览位图")
}

#[cfg(test)]
pub(crate) struct RenderedPreview {
    pub image: Arc<RenderImage>,
    pub text_regions: Vec<crate::render::text::TextGroupRegion>,
}

#[cfg(test)]
pub(crate) fn render_preview_with_regions(job: &PreviewJob) -> Result<RenderedPreview> {
    ensure_vips();
    let base = load_scaled(&job.path, job.max_edge).context("读取预览底图")?;
    let (composed, text_regions) = Photo::compose_watermark_with_regions(
        base,
        job.exif.as_ref(),
        &job.params,
        &job.text_groups,
    )
    .context("合成预览")?;
    Ok(RenderedPreview {
        image: to_render_image(&composed).context("转换预览位图")?,
        text_regions,
    })
}

/// 生成队列用的缩略图（不带水印，只是原图）。会阻塞，请在后台线程调用。
pub fn render_thumbnail(path: &Path) -> Result<Arc<RenderImage>> {
    // 缩略图和预览合成会并发跑在后台线程池里，谁先到都得先初始化 libvips。
    ensure_vips();
    let base = load_scaled(path, THUMBNAIL_MAX_EDGE).context("读取缩略图")?;
    to_render_image(&base).context("转换缩略图")
}

/// 按预览尺寸读取原图，或读取并可视化其中的 gain map。
///
/// `None` 表示输入图没有 gain map，不是解析失败。
pub fn render_gainmap_preview(path: &Path, show_gainmap: bool) -> Result<Option<Arc<RenderImage>>> {
    ensure_vips();
    let image = load_scaled(path, PREVIEW_DEFAULT_MAX_EDGE).context("读取 HDR 解析图片")?;
    if !show_gainmap {
        return to_render_image(&image).map(Some).context("转换原图预览");
    }
    let Some(gainmap) = gain_map::get_gainmap(&image) else {
        return Ok(None);
    };
    let gainmap = if gainmap.bands() == 1 {
        // `VipsImage::clone` 只会复制底层句柄；把同一个所有权句柄交给 bandjoin 后会在
        // 释放时发生重复释放。单通道 gain map 要明确创建三份独立的 vips 图像。
        let green = image_op(|out| unsafe {
            vips_sys::vips_copy(gainmap.as_ptr(), out, std::ptr::null::<i8>())
        })?;
        let blue = image_op(|out| unsafe {
            vips_sys::vips_copy(gainmap.as_ptr(), out, std::ptr::null::<i8>())
        })?;
        let mut images = [gainmap.as_ptr(), green.as_ptr(), blue.as_ptr()];
        image_op(|out| unsafe {
            vips_sys::vips_bandjoin(images.as_mut_ptr(), out, 3, std::ptr::null::<i8>())
        })?
    } else {
        gainmap
    };
    to_render_image(&gainmap)
        .map(Some)
        .context("转换 gain map 预览")
}

/// 生成彩色恢复 gain map 的黑白底图预览。会阻塞，请在后台线程调用。
pub fn render_colour_gainmap_preview(
    path: &Path,
    rotation: Rotation,
) -> Result<(Arc<RenderImage>, Arc<RenderImage>)> {
    ensure_vips();
    let image = colour_gainmap::load_black_and_white_with_colour_gainmap(path, rotation)
        .context("生成彩色恢复 Gain Map")?;
    let image = shrink_to_edge(&image, PREVIEW_DEFAULT_MAX_EDGE)?;
    let gainmap = gain_map::get_gainmap(&image).context("读取生成的彩色 Gain Map")?;
    Ok((
        to_render_image(&image).context("转换黑白底图预览")?,
        to_render_image(&gainmap).context("转换彩色 Gain Map 预览")?,
    ))
}

/// 解码视频中选作 Motion Photo 封面的帧，并转换成界面位图。
pub fn render_motion_photo_cover(
    ffmpeg: &Path,
    path: &Path,
    start: std::time::Duration,
    end: std::time::Duration,
    cover_time: std::time::Duration,
    rotation: Rotation,
) -> Result<Arc<RenderImage>> {
    ensure_vips();
    let jpeg = crate::features::motion_photo::render_motion_photo_cover(
        ffmpeg, path, start, end, cover_time, rotation,
    )?;
    let borrowed = vips::VipsImage::from_buffer(&jpeg).context("读取视频封面 JPEG")?;
    let image = unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(borrowed.as_ptr()))? };
    let image = shrink_to_edge(&image, PREVIEW_DEFAULT_MAX_EDGE)?;
    to_render_image(&image).context("转换视频封面预览")
}

/// 把任意照片写成黑白底图 + 彩色恢复 gain map 的 JPEG。
pub fn export_colour_gainmap(path: &Path, output: &Path, rotation: Rotation) -> Result<()> {
    ensure_vips();
    let image = colour_gainmap::load_black_and_white_with_colour_gainmap(path, rotation)?;
    let output_c = std::ffi::CString::new(output.to_string_lossy().as_bytes())?;
    vips::code_to_result(unsafe {
        vips_sys::vips_jpegsave(
            image.as_ptr(),
            output_c.as_ptr(),
            c"Q".as_ptr(),
            95_i32,
            c"keep".as_ptr(),
            VipsForeignKeep::VIPS_FOREIGN_KEEP_ALL,
            std::ptr::null::<i8>(),
        )
    })?;
    gain_map::mark_jpeg_gainmap(output)?;
    Ok(())
}

/// 导出输入图片中保存的原始 gain map。无 gain map 时返回 `Ok(false)`。
pub fn export_gainmap(path: &Path, output: &Path) -> Result<bool> {
    ensure_vips();
    let image = load_base_image(path)?;
    let Some(gainmap) = gain_map::get_gainmap(&image) else {
        return Ok(false);
    };
    gainmap.write_to_file(output).context("保存 gain map")?;
    Ok(true)
}

/// 读取图片并等比缩小到 `max_edge` 的方框内。
///
/// `Size::Down` 只缩小不放大：比预览尺寸还小的原图保持原样，放大只会让预览更糊。
/// `thumbnail` 会按 EXIF orientation 自动摆正，与 [`load_base_image`] 一致。
fn load_scaled(path: &Path, max_edge: i32) -> Result<VipsImage> {
    let image = VipsImage::from_file(path)?;
    image
        .thumbnail(max_edge as u32, max_edge as u32, VipsSize::VIPS_SIZE_DOWN)
        .map_err(anyhow::Error::from)
}

fn shrink_to_edge(image: &VipsImage, max_edge: i32) -> Result<VipsImage> {
    let edge = image.width().max(image.height()) as i32;
    if edge <= max_edge {
        return image_op(|out| unsafe {
            vips_sys::vips_copy(image.as_ptr(), out, std::ptr::null::<i8>())
        })
        .map_err(anyhow::Error::from);
    }
    image
        .resize(max_edge as f64 / edge as f64, None, None)
        .map_err(anyhow::Error::from)
}

/// 素材页使用与文字排版相同的解码与透明度处理。
pub(crate) fn render_logo_thumbnail(bytes: &[u8]) -> Result<Arc<RenderImage>> {
    let image = crate::media::load_logo_image(bytes)?;
    let ratio = 160.0 / image.width().max(image.height()) as f64;
    let (w, h) = (
        (image.width() as f64 * ratio).round().max(1.0) as i32,
        (image.height() as f64 * ratio).round().max(1.0) as i32,
    );
    let image = crate::render::text::scale_logo(image, w, h)?;
    to_render_image(&image)
}

/// vips 图像 → GPUI 位图。
///
/// GPUI 用 `image` crate 的 `Rgba` 缓冲承载 **BGRA** 字节序，所以这里必须交换 vips 的
/// R/B 通道，否则预览会红蓝互换。
fn to_render_image(img: &VipsImage) -> Result<Arc<RenderImage>> {
    let img = image_op(|out| unsafe {
        vips_sys::vips_cast(
            img.as_ptr(),
            out,
            VipsBandFormat::VIPS_FORMAT_UCHAR,
            std::ptr::null::<i8>(),
        )
    })
    .context("转换为 8bit 失败")?;
    let (width, height) = (img.width(), img.height());
    if width <= 0 || height <= 0 {
        bail!("尺寸非法：{width}x{height}");
    }

    let mut bytes = image_write_to_memory(&img).context("生成预览像素失败")?;
    match img.bands() {
        4 => {
            for pixel in bytes.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        3 => bytes = rgb_to_bgra(&bytes),
        bands => bail!("不支持的通道数：{bands}"),
    }

    let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width as u32, height as u32, bytes)
        .context("位图尺寸与像素数据不匹配")?;
    Ok(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

fn rgb_to_bgra(rgb: &[u8]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgb.len() / 3 * 4);
    for pixel in rgb.as_chunks::<3>().0 {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], u8::MAX]);
    }
    bgra
}

#[cfg(test)]
mod preview_region_tests {
    use super::*;
    use crate::watermark::{Placement, TextAlign, TextDirection};

    #[test]
    fn text_regions_match_visible_pixels_after_placement_padding_and_rotation() {
        let path = PathBuf::from("./test_images/DSC_4587.jpg");
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
            path: "./test_images/DSC_4587.jpg".into(),
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
    use crate::watermark::{
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
            path: PathBuf::from("test_images/ultra_hdr.jpg"),
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
