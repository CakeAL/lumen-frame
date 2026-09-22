//! 预览与缩略图的位图生成。
//!
//! 水印合成本身仍然由 [`Photo::compose_watermark`] 负责；这一层只做 UI 才需要的两件事：
//! 把底图缩到预览尺寸，以及把 libvips 的像素缓冲换成 GPUI 能直接渲染的 [`RenderImage`]。
//!
//! 这里的函数都会阻塞，只能在后台线程里调用。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use gpui_kit::RenderImage;
use image::{Frame, ImageBuffer, Rgba};
use libvips::{VipsImage, ops};

use crate::{
    features::colour_gainmap,
    gainmap as gain_map,
    media::{ExifInfo, ensure_vips, load_base_image},
    photo::Photo,
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
    let gainmap = if gainmap.get_bands() == 1 {
        // `VipsImage::clone` 只会复制底层句柄；把同一个所有权句柄交给 bandjoin 后会在
        // 释放时发生重复释放。单通道 gain map 要明确创建三份独立的 vips 图像。
        let green = ops::copy(&gainmap)?;
        let blue = ops::copy(&gainmap)?;
        ops::bandjoin(&mut [gainmap, green, blue])?
    } else {
        gainmap
    };
    to_render_image(&gainmap)
        .map(Some)
        .context("转换 gain map 预览")
}

/// 生成彩色恢复 gain map 的黑白底图预览。会阻塞，请在后台线程调用。
pub fn render_colour_gainmap_preview(path: &Path) -> Result<(Arc<RenderImage>, Arc<RenderImage>)> {
    ensure_vips();
    let image = colour_gainmap::load_black_and_white_with_colour_gainmap(path)
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
) -> Result<Arc<RenderImage>> {
    ensure_vips();
    let jpeg = crate::features::motion_photo::render_motion_photo_cover(
        ffmpeg, path, start, end, cover_time,
    )?;
    let image = VipsImage::new_from_buffer(&jpeg, "").context("读取视频封面 JPEG")?;
    let image = shrink_to_edge(&image, PREVIEW_DEFAULT_MAX_EDGE)?;
    to_render_image(&image).context("转换视频封面预览")
}

/// 把任意照片写成黑白底图 + 彩色恢复 gain map 的 JPEG。
pub fn export_colour_gainmap(path: &Path, output: &Path) -> Result<()> {
    ensure_vips();
    let image = colour_gainmap::load_black_and_white_with_colour_gainmap(path)?;
    ops::jpegsave_with_opts(
        &image,
        &output.to_string_lossy(),
        &ops::JpegsaveOptions {
            q: 95,
            keep: ops::ForeignKeep::All,
            profile: None,
            ..Default::default()
        },
    )?;
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
    ops::pngsave(&gainmap, &output.to_string_lossy()).context("保存 gain map")?;
    Ok(true)
}

/// 读取图片并等比缩小到 `max_edge` 的方框内。
///
/// `Size::Down` 只缩小不放大：比预览尺寸还小的原图保持原样，放大只会让预览更糊。
/// `thumbnail` 会按 EXIF orientation 自动摆正，与 [`load_base_image`] 一致。
fn load_scaled(path: &Path, max_edge: i32) -> Result<VipsImage> {
    ops::thumbnail_with_opts(
        &path.to_string_lossy(),
        max_edge,
        &ops::ThumbnailOptions {
            height: max_edge,
            size: ops::Size::Down,
            ..Default::default()
        },
    )
    .map_err(anyhow::Error::from)
}

fn shrink_to_edge(image: &VipsImage, max_edge: i32) -> Result<VipsImage> {
    let edge = image.get_width().max(image.get_height());
    if edge <= max_edge {
        return ops::copy(image).map_err(anyhow::Error::from);
    }
    ops::resize(image, max_edge as f64 / edge as f64).map_err(anyhow::Error::from)
}

/// vips 图像 → GPUI 位图。
///
/// GPUI 用 `image` crate 的 `Rgba` 缓冲承载 **BGRA** 字节序，所以这里必须交换 vips 的
/// R/B 通道，否则预览会红蓝互换。
fn to_render_image(img: &VipsImage) -> Result<Arc<RenderImage>> {
    let img = ops::cast(img, ops::BandFormat::Uchar).context("转换为 8bit 失败")?;
    let (width, height) = (img.get_width(), img.get_height());
    if width <= 0 || height <= 0 {
        bail!("尺寸非法：{width}x{height}");
    }

    let mut bytes = img.image_write_to_memory();
    match img.get_bands() {
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
