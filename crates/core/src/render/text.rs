use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use nom_exif::ExifDateTime;
use regex::Regex;
use reverse_geocoder_cn::{Address, ReverseGeocoder};
use vips::{Error as VipsError, Result, VipsAngle, VipsBandFormat, VipsInterpretation};

use crate::{
    media::{
        ExifInfo, Rational, image_write_to_memory,
        vips::{VipsImage, from_owned_ptr, image_op},
    },
    watermark::{
        DEFAULT_TIME_FORMAT, Placement, Text, TextAlign, TextDirection, TextGroup, TextParams,
        TextSpan, TextStyle, WatermarkParams, rich_text::style_at,
    },
};

pub type SvgString = String;

/// 合成快照中的文字组可见区域，坐标属于最终输出位图，不包含透明留白。
/// `group_ix` 只用于同一份不可变输入快照；UI 再将它对应到该请求的稳定文字组 ID。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextGroupRegion {
    pub group_ix: usize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl TextGroupRegion {
    pub(crate) fn from_layer(
        layer: &VipsImage,
        group_ix: usize,
        origin: (i32, i32),
        canvas_size: (i32, i32),
        rotation: crate::rotation::Rotation,
    ) -> anyhow::Result<Option<Self>> {
        let bytes = image_write_to_memory(layer)?;
        let width = layer.width() as usize;
        let (mut left, mut top, mut right, mut bottom) =
            (layer.width() as i32, layer.height() as i32, 0, 0);
        for (ix, pixel) in bytes.as_chunks::<4>().0.iter().enumerate() {
            if pixel[3] != 0 {
                let (x, y) = ((ix % width) as i32, (ix / width) as i32);
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
        let region = Self {
            group_ix,
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        };
        Ok(region.positioned(group_ix, origin, canvas_size, rotation))
    }

    pub(crate) fn positioned(
        self,
        group_ix: usize,
        origin: (i32, i32),
        canvas_size: (i32, i32),
        rotation: crate::rotation::Rotation,
    ) -> Option<Self> {
        // 先裁到可见画布，避免超长文字在照片外的部分仍可被点击。
        let left = (self.x + origin.0).max(0);
        let top = (self.y + origin.1).max(0);
        let right = (self.x + self.width + origin.0).min(canvas_size.0);
        let bottom = (self.y + self.height + origin.1).min(canvas_size.1);
        if right <= left || bottom <= top {
            return None;
        }
        Some(
            Self {
                group_ix,
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            }
            .rotated(rotation, canvas_size),
        )
    }

    pub(crate) fn rotated(self, rotation: crate::rotation::Rotation, (w, h): (i32, i32)) -> Self {
        use crate::rotation::Rotation;
        let (x, y, width, height) = match rotation {
            Rotation::None => (self.x, self.y, self.width, self.height),
            Rotation::Clockwise90 => (h - self.y - self.height, self.x, self.height, self.width),
            Rotation::HalfTurn => (
                w - self.x - self.width,
                h - self.y - self.height,
                self.width,
                self.height,
            ),
            Rotation::CounterClockwise90 => {
                (self.y, w - self.x - self.width, self.height, self.width)
            }
        };
        Self {
            x,
            y,
            width,
            height,
            ..self
        }
    }
}

impl Text {
    pub fn render_text(
        &self,
        exif: &ExifInfo,
        img_h: i32,
        watermark_params: &WatermarkParams,
        time_format: &str,
    ) -> Result<Option<VipsImage>> {
        crate::media::ensure_vips();
        // 行数
        let line_count = self.template.len().min(self.text_params.len());
        if line_count == 0 {
            return Ok(None);
        }

        // 解析每一行：把 `{Logo}` 拆出来，其余部分用 EXIF 模板渲染
        let segment_lines: Vec<Vec<Segment>> = self
            .template
            .iter()
            .take(line_count)
            .zip(&self.text_params)
            .map(|(t, params)| {
                parse_template(t, params, exif, time_format, &watermark_params.custom_text)
            })
            .collect();

        let mut prepared = Vec::with_capacity(line_count);
        for (segments, params) in segment_lines.iter().zip(&self.text_params) {
            // 自定义文本或编辑器中的换行仍按独立视觉行排版，每行继承原行样式。
            let mut rows = vec![Vec::new()];
            for segment in segments {
                if let Segment::Text(text, style) = segment {
                    for (ix, part) in text.split('\n').enumerate() {
                        if ix > 0 {
                            rows.push(Vec::new());
                        }
                        rows.last_mut().unwrap().push(Segment::Text(
                            part.trim_end_matches('\r').into(),
                            style.clone(),
                        ));
                    }
                } else {
                    rows.last_mut().unwrap().push(segment.clone());
                }
            }
            for row in rows {
                prepared.push(prepare_line(&row, params, img_h, exif, watermark_params)?);
            }
        }
        let width = prepared.iter().map(|line| line.width).max().unwrap_or(0);
        if width <= 0 {
            return Ok(None);
        }
        // 为紧凑行的显式行间距预留空间，压缩字体行框后不能把行距一起截掉。
        let total_h: usize = prepared
            .iter()
            .map(|line| line.height + if line.compact { line.gap } else { 0 })
            .sum();

        let mut canvas = vec![0u8; width as usize * total_h * 4];

        let mut y_off = 0i32;
        for (i, line) in prepared.iter().enumerate() {
            render_line_into(&mut canvas, width, total_h as i32, y_off, line)?;
            let row_start = y_off as usize * width as usize * 4;
            let row_end = row_start + line.height * width as usize * 4;
            if line.compact
                && let Some((_, top, _, bottom)) =
                    alpha_bounds(&canvas[row_start..row_end], width as usize)
            {
                // Logo 行只用可见内容计高，不能把字体的 ascender/descender 和行框
                // 留白再次计入边框厚度。行距只用于两行之间，首尾不添加额外空白。
                let visible_h = bottom - top;
                let gap = if i + 1 < prepared.len() { line.gap } else { 0 };
                let visible_start = row_start + top * width as usize * 4;
                let visible_end = row_start + bottom * width as usize * 4;
                canvas.copy_within(visible_start..visible_end, row_start);
                canvas[row_start + visible_h * width as usize * 4..row_end].fill(0);
                y_off += (visible_h + gap) as i32;
            } else {
                y_off += line.height as i32;
            }
        }
        let total_h = y_off as usize;
        canvas.truncate(width as usize * total_h * 4);

        let img = VipsImage::from_memory(
            canvas,
            width as u32,
            total_h as u32,
            4,
            VipsBandFormat::VIPS_FORMAT_UCHAR,
        )?;
        let img = set_rgba_srgb(&img, width, total_h as i32)?;
        Ok(Some(img))
    }
}

impl TextGroup {
    pub(crate) fn uses_custom_text(&self) -> bool {
        self.text
            .template
            .iter()
            .any(|line| line.contains("{自定义文本}"))
    }

    pub(crate) fn custom_logo_names(&self) -> Vec<String> {
        let re = Regex::new(r"\{(自定义logo[1-9][0-9]*)\}").unwrap();
        self.text
            .template
            .iter()
            .flat_map(|line| {
                re.captures_iter(line)
                    .map(|caps| caps[1].to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// 自定义文本不依赖 EXIF；没有元数据的照片仍可使用这个字段。
    pub(crate) fn render_for_photo(
        &self,
        exif: Option<&ExifInfo>,
        img_h: i32,
        params: &WatermarkParams,
    ) -> Result<Option<VipsImage>> {
        if let Some(exif) = exif {
            self.render_text(exif, img_h, params)
        } else if self.uses_custom_text() || !self.custom_logo_names().is_empty() {
            self.render_text(&ExifInfo::default(), img_h, params)
        } else {
            Ok(None)
        }
    }

    /// 渲染整组文字。竖排是对完成组内排版后的位图整体旋转，因此不会改变每行自己的
    /// `TextParams::align` 语义。
    pub fn render_text(
        &self,
        exif: &ExifInfo,
        img_h: i32,
        watermark_params: &WatermarkParams,
    ) -> Result<Option<VipsImage>> {
        let Some(image) =
            self.text
                .render_text(exif, img_h, watermark_params, &self.time_format)?
        else {
            return Ok(None);
        };

        let image = if self.direction == TextDirection::Vertical {
            image_op(|out| unsafe {
                vips_sys::vips_rot(
                    image.as_ptr(),
                    out,
                    VipsAngle::VIPS_ANGLE_D90,
                    std::ptr::null::<i8>(),
                )
            })?
        } else {
            image
        };
        let image = super::text_border::apply(image, &self.border, img_h)?;
        let padding = (img_h as f64 * self.padding.max(0.0)).round() as i32;
        if padding == 0 || !matches!(self.align, TextAlign::Left | TextAlign::Right) {
            return Ok(Some(image));
        }

        let vertical_edge = matches!(self.position, Placement::Left | Placement::Right);
        let (x, y, width, height) = if vertical_edge {
            (
                0,
                if self.align == TextAlign::Left {
                    padding
                } else {
                    0
                },
                image.width() as i32,
                image.height() as i32 + padding,
            )
        } else {
            (
                if self.align == TextAlign::Left {
                    padding
                } else {
                    0
                },
                0,
                image.width() as i32 + padding,
                image.height() as i32,
            )
        };
        let background = [0.0; 4];
        let array = unsafe { vips_sys::vips_array_double_new(background.as_ptr(), 4) };
        if array.is_null() {
            return Err(VipsError::Vips("无法创建背景色".into()));
        }
        let result = image_op(|out| unsafe {
            vips_sys::vips_embed(
                image.as_ptr(),
                out,
                x,
                y,
                width,
                height,
                c"extend".as_ptr(),
                vips::VipsExtend::VIPS_EXTEND_BACKGROUND,
                c"background".as_ptr(),
                array,
                std::ptr::null::<i8>(),
            )
        });
        unsafe { vips_sys::vips_area_unref(array.cast()) };
        Ok(Some(result?))
    }
}

/// 一行模板中被拆分出的片段。
#[derive(Debug, Clone)]
enum Segment {
    /// 需要插入相机 logo
    Logo,
    CustomLogo(String),
    /// 已经用 EXIF 渲染好的普通文本
    Text(String, TextStyle),
}

/// 把模板中的 `{Logo}` 识别出来，其余文本用 EXIF 模板渲染。
fn parse_template(
    template: &str,
    params: &TextParams,
    exif: &ExifInfo,
    time_format: &str,
    custom_text: &str,
) -> Vec<Segment> {
    let re = Regex::new(r"\{(?:Logo|自定义logo[1-9][0-9]*)\}").unwrap();
    let mut segments = Vec::new();
    let mut last = 0;

    for m in re.find_iter(template) {
        if m.start() > last {
            segments.extend(styled_template_segments(
                &template[last..m.start()],
                last,
                &params.spans,
                exif,
                time_format,
                custom_text,
            ));
        }
        segments.push(if m.as_str() == "{Logo}" {
            Segment::Logo
        } else {
            Segment::CustomLogo(m.as_str()[1..m.as_str().len() - 1].to_owned())
        });
        last = m.end();
    }

    if last < template.len() {
        segments.extend(styled_template_segments(
            &template[last..],
            last,
            &params.spans,
            exif,
            time_format,
            custom_text,
        ));
    }
    // 只清理整行末尾，Logo 前的空格属于行内间隔，不能按文本片段裁掉。
    while let Some(Segment::Text(text, _)) = segments.last_mut() {
        text.truncate(text.trim_end_matches(' ').len());
        if !text.is_empty() {
            break;
        }
        segments.pop();
    }
    segments
}

/// 字段值的字数可能因照片而异，必须在替换字段时携带模板样式，不能直接沿用输出字节索引。
fn styled_template_segments(
    template: &str,
    base: usize,
    spans: &[TextSpan],
    exif: &ExifInfo,
    time_format: &str,
    custom_text: &str,
) -> Vec<Segment> {
    let mut chars = Vec::new();
    let fields = Regex::new(r"\{([^{}]+)\}").unwrap();
    let mut last = 0;
    for caps in fields.captures_iter(template) {
        let token = caps.get(0).unwrap();
        for (offset, ch) in template[last..token.start()].char_indices() {
            chars.push((ch, style_at(spans, base + last + offset)));
        }
        let value = if &caps[1] == "自定义文本" {
            Some(custom_text.to_owned())
        } else {
            resolve_exif_key_name(&caps[1], exif, time_format)
        };
        if let Some(value) = value {
            let style = style_at(spans, base + token.start());
            chars.extend(value.chars().map(|ch| (ch, style.clone())));
        }
        last = token.end();
    }
    for (offset, ch) in template[last..].char_indices() {
        chars.push((ch, style_at(spans, base + last + offset)));
    }
    // 用户输入的行内空白按原样保留，字段替换只改变内容及其样式范围。
    let mut segments = Vec::new();
    for (ch, style) in chars {
        if let Some(Segment::Text(text, last_style)) = segments.last_mut()
            && *last_style == style
        {
            text.push(ch);
        } else {
            segments.push(Segment::Text(ch.to_string(), style));
        }
    }
    segments
}

fn append_styled_text(
    text: &mut String,
    spans: &mut Vec<TextSpan>,
    value: &str,
    style: &TextStyle,
) {
    let start = text.len();
    text.push_str(value);
    if *style != TextStyle::default() {
        spans.push(TextSpan {
            start,
            end: text.len(),
            style: style.clone(),
        });
    }
}

/// 编辑器的已解析富文本；范围属于解析后的内容，与照片排版使用同一模板解析。
pub fn render_styled_template_preview(
    template: &str,
    params: &TextParams,
    exif: &ExifInfo,
    time_format: &str,
    custom_text: &str,
) -> (String, Vec<TextSpan>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    for segment in parse_template(template, params, exif, time_format, custom_text) {
        match segment {
            Segment::Text(value, style) => {
                append_styled_text(&mut text, &mut spans, &value, &style)
            }
            Segment::Logo => text.push_str("[品牌 Logo]"),
            Segment::CustomLogo(name) => text.push_str(&format!("[{name}]")),
        }
    }
    (text, spans)
}

enum LineItem {
    Glyph {
        image: Arc<super::fonts::GlyphBitmap>,
        x: i32,
        y: i32,
        color: [u8; 4],
    },
    Logo {
        image: VipsImage,
        x: i32,
        y: i32,
    },
    Underline {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        color: [u8; 4],
    },
}

/// 已完成 GPUI 排版和字形栅格化的行，坐标包含字形外伸部分，避免斜体被裁切。
struct PreparedLine {
    items: Vec<LineItem>,
    width: i32,
    height: usize,
    align: TextAlign,
    compact: bool,
    gap: usize,
}

struct LineBuilder {
    items: Vec<LineItem>,
    x: f32,
    ascent: f32,
    descent: f32,
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

impl LineBuilder {
    fn text(
        &mut self,
        text: &str,
        spans: &[TextSpan],
        params: &TextParams,
        size: f32,
        watermark: &WatermarkParams,
    ) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let backend = super::fonts::backend()?;
        let shaped = backend.shape(text, spans, params, size, watermark);
        self.ascent = self.ascent.max(f32::from(shaped.layout.ascent));
        self.descent = self.descent.max(f32::from(shaped.layout.descent));
        for run in &shaped.layout.runs {
            for glyph in &run.glyphs {
                let style = shaped
                    .styles
                    .iter()
                    .find(|style| style.start <= glyph.index && glyph.index < style.end);
                let color = style
                    .map(|style| style.color)
                    .unwrap_or_else(|| text_color(params, watermark));
                let gx = self.x + f32::from(glyph.position.x);
                let image =
                    backend.glyph(run.font_id, glyph.id, size, gx, glyph.is_emoji, color)?;
                let x = gx.floor() as i32 + image.x;
                let y = f32::from(glyph.position.y).round() as i32 + image.y;
                self.left = self.left.min(x);
                self.right = self.right.max(x + image.width);
                self.top = self.top.min(y);
                self.bottom = self.bottom.max(y + image.height);
                self.items.push(LineItem::Glyph { image, x, y, color });
            }
        }
        for style in &shaped.styles {
            if !style.underline {
                continue;
            }
            let (offset, thickness) = backend.underline(style.font, size);
            let start = self.x + f32::from(shaped.layout.x_for_index(style.start));
            let end = self.x + f32::from(shaped.layout.x_for_index(style.end));
            let (x, y) = (start.min(end).floor() as i32, offset.floor() as i32);
            let width = (start.max(end).ceil() as i32 - x).max(0);
            let height = thickness.ceil() as i32;
            self.bottom = self.bottom.max(y + height);
            self.items.push(LineItem::Underline {
                x,
                y,
                width,
                height,
                color: style.color,
            });
        }
        self.x += f32::from(shaped.layout.width);
        Ok(())
    }
}

fn prepare_line(
    segments: &[Segment],
    params: &TextParams,
    img_h: i32,
    exif: &ExifInfo,
    watermark: &WatermarkParams,
) -> Result<PreparedLine> {
    let size = (params.size * img_h as f64).max(1.0) as f32;
    let backend = super::fonts::backend()?;
    let mut probe_text = String::new();
    let mut probe_spans = Vec::new();
    for segment in segments {
        if let Segment::Text(value, style) = segment {
            append_styled_text(&mut probe_text, &mut probe_spans, value, style);
        }
    }
    if probe_text.is_empty() {
        probe_text.push('H');
    }
    let probe = backend.shape(&probe_text, &probe_spans, params, size, watermark);
    let target_h = backend.cap_height(&probe).unwrap_or(size * 0.7).max(1.0);
    let custom_h = (params.logo_size.unwrap_or(params.size) * img_h as f64)
        .round()
        .max(1.0);
    let mut line = LineBuilder {
        items: Vec::new(),
        x: 0.0,
        ascent: 0.0,
        descent: 0.0,
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
    };
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut leading = true;
    let mut compact = false;
    for segment in segments {
        match segment {
            Segment::Text(value, style) => {
                let value = if leading {
                    value.trim_start()
                } else {
                    value.as_str()
                };
                if !value.is_empty() {
                    leading = false;
                }
                append_styled_text(&mut text, &mut spans, value, style);
            }
            Segment::Logo | Segment::CustomLogo(_) => {
                let make = exif.make.as_deref().unwrap_or_default();
                let logo = match segment {
                    Segment::CustomLogo(name) => watermark
                        .custom_logos
                        .get(name)
                        .map(|bytes| load_custom_logo(bytes, custom_h))
                        .transpose()?
                        .flatten(),
                    _ => find_make_logo(make, watermark),
                };
                if let Some(logo) = logo {
                    line.text(&text, &spans, params, size, watermark)?;
                    text.clear();
                    spans.clear();
                    let custom = matches!(segment, Segment::CustomLogo(_));
                    compact |= custom;
                    let h = if custom { custom_h } else { target_h as f64 }
                        .round()
                        .max(1.0) as i32;
                    let w = (h as f64 * logo.width() as f64 / logo.height() as f64)
                        .round()
                        .max(1.0) as i32;
                    let image = scale_logo(logo, w, h)?;
                    line.items.push(LineItem::Logo {
                        image,
                        x: line.x.round() as i32,
                        y: -h,
                    });
                    line.ascent = line.ascent.max(h as f32);
                    line.top = line.top.min(-h);
                    line.x += w as f32;
                    leading = false;
                } else if matches!(segment, Segment::Logo) {
                    let brand = clean_make_display(make);
                    if !brand.is_empty() {
                        leading = false;
                        text.push_str(&brand);
                    }
                }
            }
        }
    }
    line.text(&text, &spans, params, size, watermark)?;
    let natural_h = line.ascent + line.descent;
    let height = (size * params.line_spacing as f32).max(natural_h).max(1.0);
    let baseline = (line.ascent + (height - natural_h) / 2.0).round() as i32;
    let inset = -(baseline + line.top).min(0);
    let y_shift = baseline + inset;
    let x_shift = -line.left;
    for item in &mut line.items {
        let (LineItem::Glyph { x, y, .. }
        | LineItem::Logo { x, y, .. }
        | LineItem::Underline { x, y, .. }) = item;
        *x += x_shift;
        *y += y_shift;
    }
    Ok(PreparedLine {
        items: line.items,
        width: (line.x.ceil() as i32).max(line.right) + x_shift,
        height: (height.ceil() as i32 + inset)
            .max(line.bottom + y_shift)
            .max(1) as usize,
        align: params.align,
        compact,
        gap: (size * (params.line_spacing as f32 - 1.0).max(0.0)).round() as usize,
    })
}

pub(super) fn text_color(params: &TextParams, watermark_params: &WatermarkParams) -> [u8; 4] {
    if let Some(rgb) = params.color {
        [rgb[0], rgb[1], rgb[2], 255]
    } else if auto_color(watermark_params) {
        [0, 0, 0, 255]
    } else {
        [255, 255, 255, 255]
    }
}

fn render_line_into(
    canvas: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    y_off: i32,
    line: &PreparedLine,
) -> Result<()> {
    let x_off = match line.align {
        TextAlign::Left => 0,
        TextAlign::Center => (canvas_w - line.width) / 2,
        TextAlign::Right => canvas_w - line.width,
    };
    for item in &line.items {
        match item {
            LineItem::Glyph { image, x, y, color } => {
                for row in 0..image.height {
                    let dy = y + y_off + row;
                    if !(0..canvas_h).contains(&dy) {
                        continue;
                    }
                    for col in 0..image.width {
                        let dx = x + x_off + col;
                        if !(0..canvas_w).contains(&dx) {
                            continue;
                        }
                        let ix = (row * image.width + col) as usize;
                        // GPUI 彩色字形是 BGRA straight alpha，普通字形是单通道覆盖率。
                        let (rgb, coverage) = if image.color {
                            let pixel = &image.pixels[ix * 4..ix * 4 + 4];
                            ([pixel[2], pixel[1], pixel[0]], pixel[3])
                        } else {
                            ([color[0], color[1], color[2]], image.pixels[ix])
                        };
                        if coverage > 0 {
                            blend_pixel(canvas, canvas_w, dx, dy, rgb[0], rgb[1], rgb[2], coverage);
                        }
                    }
                }
            }
            LineItem::Logo { image, x, y } => {
                draw_logo(canvas, canvas_w, canvas_h, y_off, x + x_off, *y, image)?
            }
            LineItem::Underline {
                x,
                y,
                width,
                height,
                color,
            } => {
                for dy in (y + y_off).max(0)..(y + y_off + height).min(canvas_h) {
                    for dx in (x + x_off).max(0)..(x + x_off + width).min(canvas_w) {
                        blend_pixel(
                            canvas, canvas_w, dx, dy, color[0], color[1], color[2], color[3],
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn draw_logo(
    canvas: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    y_off: i32,
    x: i32,
    y: i32,
    logo: &VipsImage,
) -> Result<()> {
    let (logo_w, logo_h) = (logo.width() as i32, logo.height() as i32);
    let bands = logo.bands();
    if bands != 4 {
        return Err(VipsError::Vips("logo image must be RGBA".into()));
    }
    let data = logo.write_to_memory()?;
    let dx0 = x;
    let dy0 = y + y_off;

    for sy in 0..logo_h {
        let dy = dy0 + sy;
        if dy < 0 || dy >= canvas_h {
            continue;
        }
        for sx in 0..logo_w {
            let dx = dx0 + sx;
            if dx < 0 || dx >= canvas_w {
                continue;
            }
            let si = ((sy as usize * logo_w as usize) + sx as usize) * 4;
            let sa = data[si + 3] as f32 / 255.0;
            if sa <= 0.0 {
                continue;
            }
            let idx = ((dy as usize * canvas_w as usize) + dx as usize) * 4;
            let da = canvas[idx + 3] as f32 / 255.0;
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 0.0 {
                continue;
            }
            for c in 0..3 {
                canvas[idx + c] =
                    ((data[si + c] as f32 * sa + canvas[idx + c] as f32 * da * (1.0 - sa)) / out_a)
                        .round() as u8;
            }
            canvas[idx + 3] = (out_a * 255.0).round() as u8;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn blend_pixel(
    canvas: &mut [u8],
    canvas_w: i32,
    x: i32,
    y: i32,
    r: u8,
    g: u8,
    b: u8,
    coverage: u8,
) {
    let idx = ((y as usize * canvas_w as usize) + x as usize) * 4;
    let sa = coverage as f32 / 255.0;
    let da = canvas[idx + 3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    canvas[idx] = ((r as f32 * sa + canvas[idx] as f32 * da * (1.0 - sa)) / out_a).round() as u8;
    canvas[idx + 1] =
        ((g as f32 * sa + canvas[idx + 1] as f32 * da * (1.0 - sa)) / out_a).round() as u8;
    canvas[idx + 2] =
        ((b as f32 * sa + canvas[idx + 2] as f32 * da * (1.0 - sa)) / out_a).round() as u8;
    canvas[idx + 3] = (out_a * 255.0).round() as u8;
}

/// 把 libvips 图片统一转成 RGBA / uchar，用于手工合成像素。
fn to_rgba(img: VipsImage) -> Result<VipsImage> {
    let img = image_op(|out| unsafe {
        vips_sys::vips_cast(
            img.as_ptr(),
            out,
            VipsBandFormat::VIPS_FORMAT_UCHAR,
            std::ptr::null::<i8>(),
        )
    })?;
    let bands = img.bands();
    let rgba = match bands {
        4 => img,
        3 => image_op(|out| unsafe {
            vips_sys::vips_addalpha(img.as_ptr(), out, std::ptr::null::<i8>())
        })?,
        2 => {
            let grey = extract_band(&img, 0)?;
            let alpha = extract_band(&img, 1)?;
            let g2 = copy_image(&grey)?;
            let g3 = copy_image(&grey)?;
            let rgb = join_images(&[&grey, &g2, &g3])?;
            join_images(&[&rgb, &alpha])?
        }
        1 => {
            let g1 = img;
            let g2 = copy_image(&g1)?;
            let g3 = copy_image(&g1)?;
            let rgb = join_images(&[&g1, &g2, &g3])?;
            let value = [255.0];
            let alpha = unsafe {
                from_owned_ptr(vips_sys::vips_image_new_from_image(
                    rgb.as_ptr(),
                    value.as_ptr(),
                    1,
                ))?
            };
            join_images(&[&rgb, &alpha])?
        }
        _ => {
            return Err(VipsError::Vips(
                "unsupported logo bands, expected 1..=4".into(),
            ));
        }
    };

    set_rgba_srgb(&rgba, rgba.width() as i32, rgba.height() as i32)
}

pub(crate) fn scale_logo(img: VipsImage, target_w: i32, target_h: i32) -> Result<VipsImage> {
    let img = to_rgba(img)?;
    let (w, h) = (img.width() as i32, img.height() as i32);
    if (w, h) != (target_w, target_h) {
        let scale_x = target_w as f64 / w as f64;
        let scale_y = target_h as f64 / h as f64;
        return img.resize(scale_x, Some(scale_y), None);
    }
    Ok(img)
}

/// 尺寸针对素材的可见内容。SVG 先量出留白比例，再按实际需要的分辨率栅格化，
/// 避免裁掉透明边缘后又放大一张过小的位图；素材库里的原文件保持原样。
fn load_custom_logo(bytes: &[u8], height: f64) -> Result<Option<VipsImage>> {
    let natural = to_rgba(crate::media::load_logo_image(bytes)?)?;
    let pixels = natural.write_to_memory()?;
    let Some((_, top, _, bottom)) = alpha_bounds(&pixels, natural.width() as usize) else {
        return Ok(None);
    };
    let raster_h = height * natural.height() as f64 / (bottom - top) as f64;
    let raster = to_rgba(crate::media::vips::load_logo_at_height(
        bytes, natural, raster_h,
    )?)?;
    let pixels = raster.write_to_memory()?;
    let Some((left, top, right, bottom)) = alpha_bounds(&pixels, raster.width() as usize) else {
        return Ok(None);
    };
    let cropped = image_op(|out| unsafe {
        vips_sys::vips_extract_area(
            raster.as_ptr(),
            out,
            left as i32,
            top as i32,
            (right - left) as i32,
            (bottom - top) as i32,
            std::ptr::null::<i8>(),
        )
    })?;
    Ok(Some(cropped))
}

/// RGBA 的半开可见边界；全透明素材不占用排版位置。
fn alpha_bounds(pixels: &[u8], width: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut left, mut top, mut right, mut bottom) = (width, usize::MAX, 0, 0);
    for (ix, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        if pixel[3] != 0 {
            let (x, y) = (ix % width, ix / width);
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    (bottom != 0).then_some((left, top, right, bottom))
}

fn copy_image(image: &VipsImage) -> Result<VipsImage> {
    image_op(|out| unsafe { vips_sys::vips_copy(image.as_ptr(), out, std::ptr::null::<i8>()) })
}

fn extract_band(image: &VipsImage, band: i32) -> Result<VipsImage> {
    image_op(|out| unsafe {
        vips_sys::vips_extract_band(image.as_ptr(), out, band, std::ptr::null::<i8>())
    })
}

fn join_images(images: &[&VipsImage]) -> Result<VipsImage> {
    let mut raw: Vec<_> = images.iter().map(|image| image.as_ptr()).collect();
    image_op(|out| unsafe {
        vips_sys::vips_bandjoin(
            raw.as_mut_ptr(),
            out,
            raw.len() as i32,
            std::ptr::null::<i8>(),
        )
    })
}

fn set_rgba_srgb(image: &VipsImage, width: i32, height: i32) -> Result<VipsImage> {
    image_op(|out| unsafe {
        vips_sys::vips_copy(
            image.as_ptr(),
            out,
            c"width".as_ptr(),
            width,
            c"height".as_ptr(),
            height,
            c"bands".as_ptr(),
            4_i32,
            c"interpretation".as_ptr(),
            VipsInterpretation::VIPS_INTERPRETATION_sRGB,
            std::ptr::null::<i8>(),
        )
    })
}

/// 根据给定的模板生成文字。
///
/// 缺失的字段不会被保留成 `{xxx}` 字面量，而是直接丢弃，
pub fn render_exif_template(template: &str, exif: &ExifInfo, time_format: &str) -> String {
    render_watermark_template(template, exif, time_format, "")
}

/// 解析 EXIF 与当前照片的 `{自定义文本}`。自定义内容按原文插入，不再解析其中的字段。
pub fn render_watermark_template(
    template: &str,
    exif: &ExifInfo,
    time_format: &str,
    custom_text: &str,
) -> String {
    let re = Regex::new(r"\{([^{}]+)\}").unwrap();
    let mut out = String::new();
    let mut last = 0usize;

    for caps in re.captures_iter(template) {
        let whole = caps.get(0).unwrap();
        if whole.start() > last {
            out.push_str(&template[last..whole.start()]);
        }
        let key = &caps[1];
        let value = if key == "自定义文本" {
            Some(custom_text.to_owned())
        } else {
            resolve_exif_key_name(key, exif, time_format)
        };
        if let Some(value) = value
            && !value.is_empty()
        {
            out.push_str(&value);
        }
        last = whole.end();
    }
    if last < template.len() {
        out.push_str(&template[last..]);
    }
    cleanup_template(&out)
}

/// 只去掉整行末尾的空格，保留用户输入的行内间距、标点和连接符。
fn cleanup_template(s: &str) -> String {
    s.trim_end_matches(' ').to_owned()
}

fn resolve_exif_key_name(key: &str, exif: &ExifInfo, time_format: &str) -> Option<String> {
    match key {
        "拍摄日期" => exif
            .created_time
            .as_ref()
            .map(|time| format_created_time(time, time_format)),
        "品牌" => exif.make.as_deref().map(clean_make_display),
        "型号" => exif.model.as_ref().map(|m| {
            let make = exif.make.as_deref().unwrap_or_default();
            dedupe_model_brand(&format_model(m, make), make)
        }),
        "快门" => exif.exposure_time.as_ref().map(ToString::to_string),
        "光圈" => exif.f_number.as_ref().map(format_fnumber),
        "ISO" => exif.isospeed_ratings.map(|v| v.to_string()),
        "曝光补偿" => exif.exposure_bias_value.as_ref().map(ToString::to_string),
        "实际焦距" => exif.focal_length.as_ref().map(ToString::to_string),
        "白平衡" => exif.white_balance_mode.map(|v| v.to_string()),
        "等效焦距" => exif.focal_length_in35mm_film.map(|v| v.to_string()),
        "镜头生产商" => exif.lens_make.clone(),
        "镜头型号" => exif.lens_model.clone(),
        "GPS" => exif.gps_info.as_ref().and_then(format_gps),
        "海拔" => exif
            .gps_info
            .as_ref()
            .and_then(|gps| gps.altitude_meters())
            .map(format_gps_number),
        "省" | "市" | "区" => {
            administrative_address(exif.gps_info.as_ref()).map(|address| match key {
                "省" => address.province.name.clone(),
                "市" => address.city.name.clone(),
                "区" => address.county.name.clone(),
                _ => unreachable!("已在外层匹配行政区字段"),
            })
        }
        // Logo 由 render_text 的行内图像处理，这里不替换成文本
        "Logo" => None,
        _ => None,
    }
}

/// 将 EXIF 的度、分、秒坐标格式化为便于水印展示的度分文本。
fn format_gps(gps: &nom_exif::GPSInfo) -> Option<String> {
    let latitude = format_gps_coordinate(
        gps.latitude.degrees.to_f64()?,
        gps.latitude.minutes.to_f64()?,
        gps.latitude.seconds.to_f64()?,
        gps.latitude_ref.as_char(),
    )?;
    let longitude = format_gps_coordinate(
        gps.longitude.degrees.to_f64()?,
        gps.longitude.minutes.to_f64()?,
        gps.longitude.seconds.to_f64()?,
        gps.longitude_ref.as_char(),
    )?;
    Some(format!("{latitude} {longitude}"))
}

fn format_gps_coordinate(
    degrees: f64,
    minutes: f64,
    seconds: f64,
    hemisphere: char,
) -> Option<String> {
    if !(degrees.is_finite() && minutes.is_finite() && seconds.is_finite()) {
        return None;
    }
    let total_minutes = ((minutes + seconds / 60.0) * 100.0).round() / 100.0;
    if total_minutes >= 60.0 {
        return Some(format!("{:.0}°0.00'{hemisphere}", degrees + 1.0));
    }
    Some(format!("{degrees:.0}°{total_minutes:.2}'{hemisphere}"))
}

fn format_gps_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// 进程级缓存：同一张（或多张拥有相同 GPS 的）照片在模板中先后使用 `{省}`、`{市}`、
/// `{区}` 时只进行一次逆地理查询。数据源由 crate 内嵌，整个过程不会访问网络。
fn administrative_address(gps: Option<&nom_exif::GPSInfo>) -> Option<Address> {
    let gps = gps?;
    let latitude = gps.latitude_decimal()?;
    let longitude = gps.longitude_decimal()?;
    let key = (latitude.to_bits(), longitude.to_bits());

    type AddressCache = Mutex<HashMap<(u64, u64), Option<Address>>>;
    static GEOCODER: OnceLock<Option<ReverseGeocoder>> = OnceLock::new();
    static CACHE: OnceLock<AddressCache> = OnceLock::new();

    let mut cache = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(address) = cache.get(&key) {
        return address.clone();
    }

    let address = GEOCODER
        .get_or_init(|| ReverseGeocoder::embedded().ok())
        .as_ref()
        .and_then(|geocoder| geocoder.reverse_geocode(latitude, longitude));
    cache.insert(key, address.clone());
    address
}

/// 时间格式的默认值。
/// 界面上给用户参考的常用时间格式。
pub const TIME_FORMAT_EXAMPLES: &[(&str, &str)] = &[
    ("2026/09/10", "%Y/%m/%d"),
    ("2026-09-10", "%Y-%m-%d"),
    ("2026年9月10日", "%Y年%m月%d日"),
    ("09/10 14:30", "%m/%d %H:%M"),
    ("2026/09/10 14:30:05", "%Y/%m/%d %H:%M:%S"),
    ("2026-09-10 星期四", "%Y-%m-%d %A"),
];

/// 判断 strftime 模板是否可用。
///
/// 模板直接来自界面输入框，而 chrono 在写出非法模板时会 panic（`Display` 失败被
/// `to_string()` 转成了 panic），用户随手打一个孤立的 `%` 就能让导出整批崩掉。
pub fn time_format_is_valid(time_format: &str) -> bool {
    use chrono::format::{Item, StrftimeItems};

    if time_format.trim().is_empty() {
        return false;
    }
    !StrftimeItems::new(time_format).any(|item| matches!(item, Item::Error))
}

fn format_created_time(value: &ExifDateTime, time_format: &str) -> String {
    // 非法模板退回默认格式，而不是让渲染整个失败。
    let time_format = if time_format_is_valid(time_format) {
        time_format
    } else {
        DEFAULT_TIME_FORMAT
    };
    if let Some(dt) = value.aware() {
        dt.format(time_format).to_string()
    } else {
        value.into_naive().format(time_format).to_string()
    }
}

fn format_model(model: &str, make: &str) -> String {
    let make = make.replace("CORPORATION", "").trim().to_lowercase();
    if make.contains("sony") {
        model.replace("ILCE-", "α").to_lowercase()
    } else if make.contains("nikon") {
        model
            .replace(&make.to_uppercase(), "")
            .trim()
            .replace(['Z', 'z'], "ℤ")
            .split('_')
            .collect::<Vec<_>>()
            .split_last()
            .map(|(last, rest)| {
                if let Ok(num) = last.parse::<i32>() {
                    format!("{} {}", rest.join(" "), to_roman(num))
                } else {
                    format!("{} {}", rest.join(" "), last)
                }
            })
            .unwrap_or_else(|| model.trim().to_string())
    } else {
        model.to_owned()
    }
}

/// 用于展示的品牌名（去掉 “CORPORATION” 这类后缀）。
fn clean_make_display(make: &str) -> String {
    make.replace("CORPORATION", "").trim().to_string()
}

/// 如果型号以品牌名开头（例如 “Xiaomi 15” 以 “Xiaomi” 开头），
/// 就把品牌前缀去掉，避免和品牌/Logo 重复显示。
fn dedupe_model_brand(model: &str, make: &str) -> String {
    let make = clean_make_display(make);
    if make.is_empty() {
        return model.trim().to_string();
    }
    let model = model.trim();
    let make_lower = make.to_lowercase();
    if model.len() >= make_lower.len() && model[..make_lower.len()].to_lowercase() == make_lower {
        let rest = &model[make_lower.len()..];
        return rest
            .trim_start_matches(|c: char| c.is_whitespace() || c == '-' || c == '_')
            .to_string();
    }
    model.to_string()
}

fn format_fnumber(value: &Rational) -> String {
    let format_value = |v: f64| {
        format!("{v:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    };
    match value {
        Rational::Float(v) => format_value(*v),
        Rational::Fraction(n, d) => {
            let v = *n as f64 / *d as f64;
            format_value(v)
        }
    }
}

/// 取出某个品牌的标志 SVG。
///
/// 标志在编译期就嵌进二进制，而不是运行期按相对路径读 `static/`：打包成 `.app` 之后
/// 进程的工作目录不再是项目根目录，相对路径会静默失效，表现就是导出时 `{Logo}` 凭空
/// 消失 —— 而且只在别人的机器上出现。
fn find_make_logo(make: &str, watermark_params: &WatermarkParams) -> Option<VipsImage> {
    let make = make.replace("CORPORATION", "").trim().to_lowercase();
    let suffix = if auto_color(watermark_params) {
        'b'
    } else {
        'w'
    };

    // 亮色背景配黑色标志，深色背景配白色标志。
    let svg = match (make.as_str(), suffix) {
        ("canon", 'b') => include_str!("../../../../assets/logo/canon-b.svg"),
        ("canon", 'w') => include_str!("../../../../assets/logo/canon-w.svg"),
        ("dji", 'b') => include_str!("../../../../assets/logo/dji-b.svg"),
        ("dji", 'w') => include_str!("../../../../assets/logo/dji-w.svg"),
        ("fujifilm", 'b') => include_str!("../../../../assets/logo/fujifilm-b.svg"),
        ("fujifilm", 'w') => include_str!("../../../../assets/logo/fujifilm-w.svg"),
        ("hasselblad", 'b') => include_str!("../../../../assets/logo/hasselblad-b.svg"),
        ("hasselblad", 'w') => include_str!("../../../../assets/logo/hasselblad-w.svg"),
        ("leica", 'b') => include_str!("../../../../assets/logo/leica-b.svg"),
        ("leica", 'w') => include_str!("../../../../assets/logo/leica-w.svg"),
        ("nikon", 'b') => include_str!("../../../../assets/logo/nikon-b.svg"),
        ("nikon", 'w') => include_str!("../../../../assets/logo/nikon-w.svg"),
        ("olympus", 'b') => include_str!("../../../../assets/logo/olympus-b.svg"),
        ("olympus", 'w') => include_str!("../../../../assets/logo/olympus-w.svg"),
        ("panasonic", 'b') => include_str!("../../../../assets/logo/panasonic-b.svg"),
        ("panasonic", 'w') => include_str!("../../../../assets/logo/panasonic-w.svg"),
        ("pentax", 'b') => include_str!("../../../../assets/logo/pentax-b.svg"),
        ("pentax", 'w') => include_str!("../../../../assets/logo/pentax-w.svg"),
        ("ricoh", 'b') => include_str!("../../../../assets/logo/ricoh-b.svg"),
        ("ricoh", 'w') => include_str!("../../../../assets/logo/ricoh-w.svg"),
        ("sigma", 'b') => include_str!("../../../../assets/logo/sigma-b.svg"),
        ("sigma", 'w') => include_str!("../../../../assets/logo/sigma-w.svg"),
        ("songdian", 'b') => include_str!("../../../../assets/logo/songdian-b.svg"),
        ("songdian", 'w') => include_str!("../../../../assets/logo/songdian-w.svg"),
        ("sony", 'b') => include_str!("../../../../assets/logo/sony-b.svg"),
        ("sony", 'w') => include_str!("../../../../assets/logo/sony-w.svg"),
        _ => return None,
    };
    let borrowed = vips::VipsImage::from_buffer(svg.as_bytes()).ok()?;
    unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(borrowed.as_ptr())).ok() }
}

/// 浅色纯色背景或渐变中点使用黑色自动文字；其它背景使用白色。
fn auto_color(watermark_params: &WatermarkParams) -> bool {
    watermark_params
        .text_background_color()
        .is_some_and(|[red, green, blue]| !is_dark_color(red, green, blue))
}

fn is_dark_color(red: u8, green: u8, blue: u8) -> bool {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
    luminance < 0.5
}

fn to_roman(mut number: i32) -> String {
    let values = [1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1];
    let symbols = [
        "M", "CM", "D", "CD", "C", "XC", "L", "XL", "X", "IX", "V", "IV", "I",
    ];
    let mut result = String::new();
    for (index, value) in values.into_iter().enumerate() {
        while number >= value {
            result.push_str(symbols[index]);
            number -= value;
        }
    }
    result
}

#[cfg(test)]
mod time_format_tests {
    use super::*;

    #[test]
    fn recognises_valid_and_invalid_templates() {
        assert!(time_format_is_valid("%Y/%m/%d"));
        assert!(time_format_is_valid("%Y年%m月%d日 %H:%M"));
        // 孤立的 `%` 是界面输入框里最容易敲出来的非法模板。
        assert!(!time_format_is_valid("%"));
        assert!(!time_format_is_valid("%Y/%m/%d %"));
        assert!(!time_format_is_valid("   "));
    }

    #[test]
    fn invalid_template_falls_back_instead_of_panicking() {
        let exif = ExifInfo::read(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_images/DSC_4587.jpg"
        )))
        .unwrap();
        assert!(exif.created_time.is_some(), "测试图缺少拍摄时间");

        let fallback = render_exif_template("{拍摄日期}", &exif, DEFAULT_TIME_FORMAT);

        // chrono 写出非法模板时会 panic；这里必须退回默认格式而不是崩掉整批导出。
        assert_eq!(render_exif_template("{拍摄日期}", &exif, "%"), fallback);
        assert_eq!(
            render_exif_template("{拍摄日期}", &exif, "%Y/%m/%d %"),
            fallback
        );

        // 合法模板照常生效，说明上面比的不是「反正都一样」。
        assert_ne!(
            render_exif_template("{拍摄日期}", &exif, "%Y-%m-%d"),
            fallback
        );
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;
    use crate::{media::ensure_vips, rotation::Rotation};

    #[test]
    fn transparent_padding_and_clipped_content_are_excluded_from_the_region() {
        ensure_vips();
        let mut bytes = vec![0_u8; 10 * 8 * 4];
        for y in 1..4 {
            for x in 2..6 {
                bytes[(y * 10 + x) * 4 + 3] = 255;
            }
        }
        let layer =
            VipsImage::from_memory(bytes, 10, 8, 4, VipsBandFormat::VIPS_FORMAT_UCHAR).unwrap();
        let region = TextGroupRegion::from_layer(&layer, 7, (-3, 2), (20, 10), Rotation::None)
            .unwrap()
            .unwrap();
        assert_eq!(
            region,
            TextGroupRegion {
                group_ix: 7,
                x: 0,
                y: 3,
                width: 3,
                height: 3
            }
        );
        let rotated =
            TextGroupRegion::from_layer(&layer, 7, (-3, 2), (20, 10), Rotation::Clockwise90)
                .unwrap()
                .unwrap();
        assert_eq!(
            rotated,
            TextGroupRegion {
                group_ix: 7,
                x: 4,
                y: 0,
                width: 3,
                height: 3
            }
        );
        assert!(
            TextGroupRegion::from_layer(&layer, 7, (-30, 2), (20, 10), Rotation::None)
                .unwrap()
                .is_none()
        );
        let blank = VipsImage::from_memory(
            vec![0_u8; 10 * 8 * 4],
            10,
            8,
            4,
            VipsBandFormat::VIPS_FORMAT_UCHAR,
        )
        .unwrap();
        assert!(
            TextGroupRegion::from_layer(&blank, 0, (0, 0), (20, 10), Rotation::None)
                .unwrap()
                .is_none()
        );
    }
}
