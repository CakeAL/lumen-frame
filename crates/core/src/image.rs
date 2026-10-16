//! 与 UI 无关的图片预览、缩略图、Gain Map 和 Motion Photo 媒体操作。

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
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
pub fn render_preview(job: &PreviewJob) -> Result<RasterImage> {
    ensure_vips();
    let base = load_scaled(&job.path, job.max_edge).context("读取预览底图")?;
    let params = crate::features::photo_palette::resolve(&job.path, &job.params)?;
    let composed = Photo::compose_watermark(base, job.exif.as_ref(), &params, &job.text_groups)
        .context("合成预览")?;
    rasterize(&composed).context("转换预览位图")
}

#[cfg(any(test, feature = "test-support"))]
pub struct RenderedPreview {
    pub image: RasterImage,
    pub text_regions: Vec<crate::render::text::TextGroupRegion>,
}

#[cfg(any(test, feature = "test-support"))]
pub fn render_preview_with_regions(job: &PreviewJob) -> Result<RenderedPreview> {
    ensure_vips();
    let base = load_scaled(&job.path, job.max_edge).context("读取预览底图")?;
    let params = crate::features::photo_palette::resolve(&job.path, &job.params)?;
    let (composed, text_regions) =
        Photo::compose_watermark_with_regions(base, job.exif.as_ref(), &params, &job.text_groups)
            .context("合成预览")?;
    Ok(RenderedPreview {
        image: rasterize(&composed).context("转换预览位图")?,
        text_regions,
    })
}

/// 生成队列用的缩略图（不带水印，只是原图）。会阻塞，请在后台线程调用。
pub fn render_thumbnail(path: &Path) -> Result<RasterImage> {
    // 缩略图和预览合成会并发跑在后台线程池里，谁先到都得先初始化 libvips。
    ensure_vips();
    let base = load_scaled(path, THUMBNAIL_MAX_EDGE).context("读取缩略图")?;
    rasterize(&base).context("转换缩略图")
}

/// 按预览尺寸读取原图，或读取并可视化其中的 gain map。
///
/// `None` 表示输入图没有 gain map，不是解析失败。
pub fn render_gainmap_preview(path: &Path, show_gainmap: bool) -> Result<Option<RasterImage>> {
    ensure_vips();
    let image = load_scaled(path, PREVIEW_DEFAULT_MAX_EDGE).context("读取 HDR 解析图片")?;
    if !show_gainmap {
        return rasterize(&image).map(Some).context("转换原图预览");
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
    rasterize(&gainmap).map(Some).context("转换 gain map 预览")
}

/// 生成彩色恢复 gain map 的黑白底图预览。会阻塞，请在后台线程调用。
pub fn render_colour_gainmap_preview(
    path: &Path,
    rotation: Rotation,
) -> Result<(RasterImage, RasterImage)> {
    ensure_vips();
    let image = colour_gainmap::load_black_and_white_with_colour_gainmap(path, rotation)
        .context("生成彩色恢复 Gain Map")?;
    let image = shrink_to_edge(&image, PREVIEW_DEFAULT_MAX_EDGE)?;
    let gainmap = gain_map::get_gainmap(&image).context("读取生成的彩色 Gain Map")?;
    Ok((
        rasterize(&image).context("转换黑白底图预览")?,
        rasterize(&gainmap).context("转换彩色 Gain Map 预览")?,
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
) -> Result<RasterImage> {
    ensure_vips();
    let jpeg = crate::features::motion_photo::render_motion_photo_cover(
        ffmpeg, path, start, end, cover_time, rotation,
    )?;
    let borrowed = vips::VipsImage::from_buffer(&jpeg).context("读取视频封面 JPEG")?;
    let image = unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(borrowed.as_ptr()))? };
    let image = shrink_to_edge(&image, PREVIEW_DEFAULT_MAX_EDGE)?;
    rasterize(&image).context("转换视频封面预览")
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
pub fn load_scaled(path: &Path, max_edge: i32) -> Result<VipsImage> {
    ensure_vips();
    anyhow::ensure!(
        (1..=8192).contains(&max_edge),
        "预览尺寸必须在 1 至 8192 之间"
    );
    let image = VipsImage::from_file(path)?;
    image
        .thumbnail(max_edge as u32, max_edge as u32, VipsSize::VIPS_SIZE_DOWN)
        .map_err(anyhow::Error::from)
}

pub(crate) fn shrink_to_edge(image: &VipsImage, max_edge: i32) -> Result<VipsImage> {
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
pub fn render_logo_thumbnail(bytes: &[u8]) -> Result<RasterImage> {
    let image = crate::media::load_logo_image(bytes)?;
    let ratio = 160.0 / image.width().max(image.height()) as f64;
    let (w, h) = (
        (image.width() as f64 * ratio).round().max(1.0) as i32,
        (image.height() as f64 * ratio).round().max(1.0) as i32,
    );
    let image = crate::render::text::scale_logo(image, w, h)?;
    rasterize(&image)
}

/// 媒体层的标准 RGBA 像素，不携带窗口或 GPU 纹理身份。
#[derive(Clone, Debug)]
pub struct RasterImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RasterImage {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        image::save_buffer(
            path,
            &self.pixels,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )
        .context("保存预览图片失败")
    }
}

pub(crate) fn rasterize(img: &VipsImage) -> Result<RasterImage> {
    ensure_vips();
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
    if width == 0 || height == 0 {
        bail!("尺寸非法：{width}x{height}");
    }
    let mut pixels = image_write_to_memory(&img).context("生成预览像素失败")?;
    match img.bands() {
        4 => {}
        3 => {
            pixels = pixels
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect()
        }
        bands => bail!("不支持的通道数：{bands}"),
    }
    Ok(RasterImage {
        width,
        height,
        pixels,
    })
}

pub mod layers;
