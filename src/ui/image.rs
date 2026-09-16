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
    params::WatermarkParams,
    photo::{ExifInfo, Photo, ensure_vips},
    process::text::TextGroup,
};

/// 预览底图的长边上限（px）。
///
/// 预览要的是看清效果，不是像素级还原：底图缩到这个尺寸后，合成耗时基本与原图大小
/// 无关，拖动滑块才跟得上手。
pub const PREVIEW_MAX_EDGE: i32 = 1600;

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
}

/// 按预览分辨率渲染水印照片。会阻塞，请在后台线程调用。
pub fn render_preview(job: &PreviewJob) -> Result<Arc<RenderImage>> {
    ensure_vips();
    let base = load_scaled(&job.path, PREVIEW_MAX_EDGE).context("读取预览底图")?;
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

/// 读取图片并等比缩小到 `max_edge` 的方框内。
///
/// `Size::Down` 只缩小不放大：比预览尺寸还小的原图保持原样，放大只会让预览更糊。
/// `thumbnail` 会按 EXIF orientation 自动摆正，与 [`Photo::load_base_image`] 一致。
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
