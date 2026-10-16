//! 复用应用的 GPUI 平台字体系统；后台仅持有 Send + Sync 的字体接口。

use crate::watermark::{TextParams, TextSpan, WatermarkParams, rich_text::style_at};
use gpui_kit::{
    FontId, FontRun, FontStyle, FontWeight, LineLayout, PlatformTextSystem, RenderGlyphParams,
    SUBPIXEL_VARIANTS_X, TextSystem, font, point, px, rgb,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

static BACKEND: OnceLock<FontBackend> = OnceLock::new();

/// 在创建应用的主线程上调用一次，传入与 GPUI 界面共用的 `Platform::text_system()`。
/// 独立图像调用方也须先在主线程完成初始化；不能在后台临时创建平台或窗口。
pub fn init_font_backend(platform: Arc<dyn PlatformTextSystem>) {
    BACKEND.get_or_init(|| FontBackend {
        fonts: TextSystem::new(platform.clone()),
        platform,
        glyphs: Mutex::new(GlyphCache::default()),
    });
}

pub(super) fn backend() -> vips::Result<&'static FontBackend> {
    BACKEND.get().ok_or_else(|| {
        vips::Error::Vips("字体后端尚未初始化，请在主线程调用 render::init_font_backend".into())
    })
}

pub(super) struct FontBackend {
    platform: Arc<dyn PlatformTextSystem>,
    fonts: TextSystem,
    glyphs: Mutex<GlyphCache>,
}

#[derive(Default)]
struct GlyphCache {
    bytes: usize,
    images: HashMap<RenderGlyphParams, Arc<GlyphBitmap>>,
}

pub(super) struct GlyphBitmap {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub color: bool,
    pub pixels: Vec<u8>,
}

#[derive(Clone, PartialEq)]
pub(super) struct PaintRun {
    pub start: usize,
    pub end: usize,
    pub font: FontId,
    pub color: [u8; 4],
    pub underline: bool,
}

pub(super) struct ShapedText {
    pub layout: LineLayout,
    pub styles: Vec<PaintRun>,
}

impl FontBackend {
    pub(super) fn shape(
        &self,
        text: &str,
        spans: &[TextSpan],
        params: &TextParams,
        size: f32,
        watermark: &WatermarkParams,
    ) -> ShapedText {
        let mut styles: Vec<PaintRun> = Vec::new();
        for (offset, ch) in text.char_indices() {
            let style = style_at(spans, offset);
            let family = style
                .font
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| params.resolved_font(&watermark.default_font));
            let mut descriptor = font(family.to_owned());
            descriptor.weight = if style.bold.unwrap_or(params.bold) {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            };
            descriptor.style = if style.italic.unwrap_or(params.italic) {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            };
            let color = style
                .color
                .or(params.color)
                .map(|[r, g, b]| [r, g, b, 255])
                .unwrap_or_else(|| super::text::text_color(params, watermark));
            let run = PaintRun {
                start: offset,
                end: offset + ch.len_utf8(),
                font: self.fonts.resolve_font(&descriptor),
                color,
                underline: style.underline.unwrap_or(params.underline),
            };
            if let Some(last) = styles.last_mut()
                && last.font == run.font
                && last.color == run.color
                && last.underline == run.underline
            {
                last.end = run.end;
            } else {
                styles.push(run);
            }
        }
        let runs: Vec<_> = styles
            .iter()
            .map(|run| FontRun {
                len: run.end - run.start,
                font_id: run.font,
            })
            .collect();
        ShapedText {
            layout: self.platform.layout_line(text, px(size), &runs),
            styles,
        }
    }

    pub(super) fn cap_height(&self, text: &ShapedText) -> Option<f32> {
        text.layout.runs.first().map(|run| {
            f32::from(
                self.platform
                    .font_metrics(run.font_id)
                    .cap_height(text.layout.font_size),
            )
        })
    }

    pub(super) fn underline(&self, font: FontId, size: f32) -> (f32, f32) {
        let metrics = self.platform.font_metrics(font);
        (
            -f32::from(metrics.underline_position(px(size))),
            f32::from(metrics.underline_thickness(px(size)))
                .abs()
                .max(1.0),
        )
    }

    pub(super) fn glyph(
        &self,
        font: FontId,
        glyph: gpui_kit::GlyphId,
        size: f32,
        x: f32,
        emoji: bool,
        color: [u8; 4],
    ) -> vips::Result<Arc<GlyphBitmap>> {
        let [r, g, b, _] = color;
        let params = RenderGlyphParams {
            font_id: font,
            glyph_id: glyph,
            font_size: px(size),
            subpixel_variant: point(
                (x.rem_euclid(1.0) * SUBPIXEL_VARIANTS_X as f32).floor() as u8
                    % SUBPIXEL_VARIANTS_X,
                0,
            ),
            scale_factor: 1.0,
            is_emoji: emoji,
            subpixel_rendering: false,
            dilation: if emoji {
                0
            } else {
                self.platform.glyph_dilation_for_color(
                    rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into(),
                )
            },
        };
        if let Some(image) = self
            .glyphs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .images
            .get(&params)
        {
            return Ok(image.clone());
        }
        let result = (|| -> anyhow::Result<_> {
            let bounds = self.platform.glyph_raster_bounds(&params)?;
            if bounds.size.width.0 <= 0 || bounds.size.height.0 <= 0 {
                return Ok(GlyphBitmap {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    color: emoji,
                    pixels: Vec::new(),
                });
            }
            let (size, pixels) = self.platform.rasterize_glyph(&params, bounds)?;
            anyhow::ensure!(
                pixels.len()
                    == size.width.0 as usize * size.height.0 as usize * if emoji { 4 } else { 1 },
                "字体后端返回了不支持的像素格式"
            );
            Ok(GlyphBitmap {
                x: bounds.origin.x.0,
                y: bounds.origin.y.0,
                width: size.width.0,
                height: size.height.0,
                color: emoji,
                pixels,
            })
        })()
        .map_err(|error| vips::Error::Vips(format!("GPUI 字形渲染失败：{error:#}")))?;
        let image = Arc::new(result);
        // GPUI 的窗口字形图集不对外开放；导出缓存只存 CPU 像素，限制为 16 MiB。
        const LIMIT: usize = 16 * 1024 * 1024;
        if image.pixels.len() <= LIMIT {
            let mut cache = self.glyphs.lock().unwrap_or_else(|e| e.into_inner());
            if cache.bytes + image.pixels.len() > LIMIT {
                cache.images.clear();
                cache.bytes = 0;
            }
            if !cache.images.contains_key(&params) {
                cache.bytes += image.pixels.len();
                cache.images.insert(params, image.clone());
            }
        }
        Ok(image)
    }
}

#[cfg(test)]
#[ctor::ctor(unsafe)]
fn init_test_fonts() {
    // 测试线程不能创建 macOS 平台，须在测试运行器启动工作线程前取得字体接口。
    init_font_backend(gpui_kit::platform::current_platform(true).text_system());
}
