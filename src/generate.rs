use anyhow::{Context, Result};
use libvips::VipsApp;
use libvips::ops::{self, BlendMode, CompassDirection, Extend, Interpretation};
use nom_exif::MediaParser;

use crate::params::*;

pub async fn generate_watermark(params: &WatermarkParams) -> Result<()> {
    // 初始化 libvips。
    let _app = VipsApp::default("lumen-frame").context("failed to init libvips")?;

    // 载入并按 EXIF 方向摆正原图。
    let img = ops::jpegload_with_opts(
        &params.input_path.to_string_lossy(),
        &ops::JpegloadOptions {
            autorotate: true,
            ..Default::default()
        },
    )
    .context("failed to load image")?;

    let img_w = img.get_width();
    let img_h = img.get_height();

    // 计算带边框画布尺寸（宽高分别乘以 1 + 比例）。
    let canvas_h = (img_h as f64 * (1.0 + params.border_ratio.1)).round() as i32;
    let canvas_w = (img_w as f64 * (1.0 + params.border_ratio.0)).round() as i32;

    // 把原图居中放到画布上。
    let [br, bg, bb] = params.background;
    let gravity_opts = ops::GravityOptions {
        extend: Extend::Background,
        background: vec![br as f64, bg as f64, bb as f64],
    };
    let mut canvas = ops::gravity_with_opts(
        &img,
        CompassDirection::Centre,
        canvas_w,
        canvas_h,
        &gravity_opts,
    )
    .context("failed to place image")?;

    // 从 parse 模块读取 Exif（尽力而为），提取并渲染底部水印。
    let mut parser = MediaParser::new();
    let caption = match dump_exif(&mut parser, &params.input_path).await {
        Ok(Some(exif)) => extract_capture_info(&exif).to_caption(),
        _ => None,
    };

    if let Some(caption) = caption {
        let text = render_caption(&caption, params, img_h, canvas_h)?;
        let tw = text.get_width();
        let th = text.get_height();

        // 水平居中；垂直方向居中于底部边框区域内。
        let h_extra = canvas_h - img_h;
        let border_top = h_extra / 2;
        let border_bottom = h_extra - border_top;
        let x = (canvas_w - tw) / 2;
        let y = img_h + border_top + ((border_bottom - th) / 2).max(0);

        let composite_opts = ops::Composite2Options {
            x,
            y,
            compositing_space: Interpretation::Srgb,
            premultiplied: false,
        };
        // composite2 要求两张图 band 数一致：给画布补一个不透明 alpha，与 4-band 文字对齐。
        let canvas_rgba = ops::addalpha(&canvas).context("failed to add alpha")?;
        let composed =
            ops::composite2_with_opts(&canvas_rgba, &text, BlendMode::Over, &composite_opts)
                .context("failed to composite caption")?;

        // composite2 之后带上了 alpha 通道，写出 JPEG 前先压平为 RGB。
        let flatten_opts = ops::FlattenOptions {
            background: vec![br as f64, bg as f64, bb as f64],
            ..Default::default()
        };
        canvas =
            ops::flatten_with_opts(&composed, &flatten_opts).context("failed to flatten image")?;
    }

    // 写出 JPEG。
    ops::jpegsave_with_opts(
        &canvas,
        &params.output_path.to_string_lossy(),
        &ops::JpegsaveOptions {
            q: params.quality,
            ..Default::default()
        },
    )
    .context("failed to save image")?;

    Ok(())
}

/// 渲染底部水印文字为带 alpha 的 RGBA 图像。
///
/// 注意：`libvips` crate 的 `text_with_opts` 会把只读的 `autofit-dpi` 输出属性
/// 当成输入传入，与 libvips 8.18 不兼容，会导致段错误。因此这里改用
/// `ops::text`（渲染成 1-band 白字黑底掩码）→ `resize` 放大 → 上色并合成 alpha。
fn render_caption(
    caption: &str,
    params: &WatermarkParams,
    img_h: i32,
    canvas_h: i32,
) -> Result<libvips::VipsImage> {
    // 用默认字体渲染成 1-band 掩码（白色文字、黑色背景）。
    let small = ops::text(caption).context("failed to render text")?;

    // 放大到目标高度：约为底部边框高度的 55%，并限制在合理范围内。
    let border = ((canvas_h - img_h) / 2).max(1);
    let target_h = ((border as f64) * 0.55).round().clamp(12.0, 400.0) as i32;
    let scale = target_h as f64 / small.get_height() as f64;
    let mask = if (scale - 1.0).abs() > 0.01 {
        ops::resize(&small, scale).context("failed to resize text")?
    } else {
        small
    };

    // 上色并合成 alpha：RGB 用目标颜色，alpha 用文字掩码。
    let [tr, tg, tb] = params.text_params.color;
    let color = libvips::VipsImage::new_from_image(&mask, &[tr as f64, tg as f64, tb as f64])
        .context("failed to build text color")?;
    let mut bands = [color, mask];
    let rgba = ops::bandjoin(&mut bands).context("failed to build rgba text")?;

    // bandjoin 产出的 interpretation 为 multiband，标记为 sRGB 以便 composite2 合成。
    let copy_opts = ops::CopyOptions {
        interpretation: Interpretation::Srgb,
        ..Default::default()
    };
    ops::copy_with_opts(&rgba, &copy_opts).context("failed to set interpretation")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端到端测试：读取真实照片，生成带边框与拍摄参数水印的输出。
    #[tokio::test]
    async fn generate_watermark_to_file() {
        let input = "./test_images/DSC_4587.jpg";
        let output = "./test_images/DSC_4587_watermark.jpg";

        let params = WatermarkParams::new(input, output);
        generate_watermark(&params)
            .await
            .expect("generate watermark");

        let out = std::path::Path::new(output);
        assert!(out.exists(), "output image not written: {output}");
        assert!(out.metadata().unwrap().len() > 0, "output image is empty");
    }
}
