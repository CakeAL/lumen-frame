use std::borrow::Cow;

use libvips::error::Error as VipsError;
use libvips::{Result, VipsImage, ops};
use nom_exif::ExifDateTime;
use parley::style::{FontFamily, FontStyle, FontWeight, LineHeight, StyleProperty};
use parley::{
    Alignment, AlignmentOptions, FontContext, InlineBox, InlineBoxKind, Layout, LayoutContext,
    PositionedLayoutItem,
};
use regex::Regex;
use swash::FontRef;
use swash::scale::image::{Content, Image};
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;

use crate::{
    Position,
    helper::auto_color,
    params::WatermarkParams,
    photo::{ExifInfo, Rational},
};

pub type SvgString = String;

/// 需要渲染的多行文本
#[derive(Debug, Clone)]
pub struct Text {
    /// 每行文本模板
    pub template: Vec<String>,
    /// 每行文本参数
    pub text_params: Vec<TextParams>,
    /// 文本位置
    pub position: Position,
    /// 时间格式
    pub time_format: String,
}

impl Text {
    /// 计算多行文本占用高度px，用于计算有字体一侧的margin宽度
    /// 返回 (文本相对于图片的位置, 文本高度)
    pub fn cal_height(&self, img_h: i32) -> (Position, i32) {
        if self.text_params.is_empty() {
            return (Position::Bottom, 0);
        }
        let img_h = img_h as f64;
        let text_height: i32 = self
            .text_params
            .iter()
            .map(|text_params| (text_params.size * img_h).round() as i32)
            .sum();
        let spacing: i32 = self
            .text_params
            .windows(2)
            .map(|lines| (lines[0].size * img_h * (lines[0].line_spacing - 1.0)).round() as i32)
            .sum();
        (self.position, text_height + spacing)
    }

    pub fn render_text(
        &self,
        exif: &ExifInfo,
        img_h: i32,
        watermark_params: &WatermarkParams,
    ) -> Result<Option<VipsImage>> {
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
            .map(|t| parse_template(t, exif, &self.time_format))
            .collect();

        // 计算每一行的字号
        let font_sizes: Vec<f32> = self
            .text_params
            .iter()
            .take(line_count)
            .map(|params| (params.size * img_h as f64) as f32)
            .collect();

        let mut font_ctx = FontContext::new();
        let mut layout_ctx = LayoutContext::<peniko::Brush>::new();

        let mut prepared = Vec::with_capacity(line_count);
        let mut total_h = 0usize;

        for i in 0..line_count {
            let line = prepare_line(
                &segment_lines[i],
                &self.text_params[i],
                font_sizes[i],
                exif,
                watermark_params,
                &mut font_ctx,
                &mut layout_ctx,
            )?;
            total_h += line.height;
            prepared.push(line);
        }

        // 导出图片宽度取所有行中最长一行的内容宽度，左右不留空白。
        let width = prepared
            .iter()
            .map(|line| line.layout.width().ceil() as i32)
            .max()
            .unwrap_or(0);
        if width <= 0 {
            return Ok(None);
        }

        // 用最长行宽度作为排版宽度重新换行并执行对齐：
        // 左对齐时所有行都贴左，图片宽度 = 最长行；居中/右对齐也都在该宽度内。
        for (i, line) in prepared.iter_mut().enumerate() {
            line.layout.break_all_lines(Some(width as f32));
            line.layout.align(
                alignment_for(self.text_params[i].align),
                AlignmentOptions::default(),
            );
        }

        let mut canvas = vec![0u8; width as usize * total_h * 4];

        let mut y_off = 0i32;
        for line in &prepared {
            render_line_into(&mut canvas, width, total_h as i32, y_off, line)?;
            y_off += line.height as i32;
        }

        let img = VipsImage::new_from_memory_copy(
            &canvas,
            width,
            total_h as i32,
            4,
            ops::BandFormat::Uchar,
        )?;
        let img = ops::copy_with_opts(
            &img,
            &ops::CopyOptions {
                width,
                height: total_h as i32,
                bands: 4,
                interpretation: ops::Interpretation::Srgb,
                ..Default::default()
            },
        )?;
        Ok(Some(img))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TextDirection {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub struct TextParams {
    pub font: String,
    /// 该尺寸系与图片背景高度的百分比，默认为0.03：如果图片高度1000px，那么字体高度为30px
    pub size: f64,
    /// 行间距，默认为1.3，即字体高度为30px，那么行间距是9px
    pub line_spacing: f64,
    /// None的时候即自动颜色
    pub color: Option<[u8; 3]>,
    pub italic: bool,
    pub bold: bool,
    pub align: TextAlign,
    /// 文字方向
    pub direction: TextDirection,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            font: "Arial".to_string(),
            size: 0.03,
            line_spacing: 1.3,
            color: None,
            italic: false,
            bold: false,
            align: TextAlign::default(),
            direction: TextDirection::default(),
        }
    }
}

/// 一行模板中被拆分出的片段。
#[derive(Debug, Clone)]
enum Segment {
    /// 需要插入相机 logo
    Logo,
    /// 已经用 EXIF 渲染好的普通文本
    Text(String),
}

/// 把模板中的 `{Logo}` 识别出来，其余文本用 EXIF 模板渲染。
fn parse_template(template: &str, exif: &ExifInfo, time_format: &str) -> Vec<Segment> {
    let re = Regex::new(r"\{Logo\}").unwrap();
    let mut segments = Vec::new();
    let mut last = 0;

    for m in re.find_iter(template) {
        if m.start() > last {
            let text = render_exif_template(&template[last..m.start()], exif, time_format);
            if !text.is_empty() {
                segments.push(Segment::Text(text));
            }
        }
        segments.push(Segment::Logo);
        last = m.end();
    }

    if last < template.len() {
        let text = render_exif_template(&template[last..], exif, time_format);
        if !text.is_empty() {
            segments.push(Segment::Text(text));
        }
    }
    segments
}

/// 一个已加载的相机 logo（已经缩放到目标尺寸、RGBA/uchar）。
struct LogoSlot {
    image: VipsImage,
}

/// 一行渲染所需的全部中间结果。
struct PreparedLine {
    layout: Layout<peniko::Brush>,
    slots: Vec<LogoSlot>,
    height: usize,
}

fn prepare_line(
    segments: &[Segment],
    params: &TextParams,
    font_size: f32,
    exif: &ExifInfo,
    watermark_params: &WatermarkParams,
    font_ctx: &mut FontContext,
    layout_ctx: &mut LayoutContext<peniko::Brush>,
) -> Result<PreparedLine> {
    // 先用占位字符构建一次纯文本布局，量出该行文字的实际字形高度（cap height），
    // 让 Logo 高度与文字高度相等，而不是用整个 em 高度（那样会显得比字高）。
    let mut probe_text = String::new();
    for segment in segments {
        match segment {
            Segment::Text(text) => probe_text.push_str(text),
            Segment::Logo => probe_text.push('\u{200b}'),
        }
    }
    let probe_layout = build_parley_layout(
        &probe_text,
        params,
        font_size,
        None,
        &[],
        watermark_params,
        font_ctx,
        layout_ctx,
    );
    let target_h = measure_cap_height(&probe_layout)
        .unwrap_or(font_size * 0.7)
        .max(1.0);

    let mut layout_text = String::new();
    let mut boxes: Vec<InlineBox> = Vec::new();
    let mut slots: Vec<LogoSlot> = Vec::new();
    let mut next_id = 0u64;

    for segment in segments {
        match segment {
            Segment::Text(text) => layout_text.push_str(text),
            Segment::Logo => {
                let make = exif.make.as_deref().unwrap_or_default();
                let index = layout_text.len();
                if let Some(logo) = find_make_logo(make, watermark_params) {
                    // 用零宽空格占据位置，真正的 logo 由 InlineBox 承载
                    layout_text.push('\u{200b}');

                    let (logo_w, logo_img_h) = (logo.get_width(), logo.get_height());
                    if logo_w <= 0 || logo_img_h <= 0 {
                        continue;
                    }
                    let aspect = logo_w as f64 / logo_img_h as f64;
                    // logo 高度与该行文字实际高度（cap height）一致
                    let box_h = target_h;
                    let box_w = ((box_h as f64 * aspect).round() as f32).max(1.0);
                    let logo = scale_logo(logo, box_w.round() as i32, box_h.round() as i32)?;

                    let id = next_id;
                    next_id += 1;
                    boxes.push(InlineBox {
                        id,
                        kind: InlineBoxKind::InFlow,
                        index,
                        width: box_w,
                        height: box_h,
                    });
                    slots.push(LogoSlot { image: logo });
                } else {
                    // 没有对应的 Logo 时，直接用品牌名（Make）兜底显示
                    let brand = clean_make_display(make);
                    if !brand.is_empty() {
                        layout_text.push_str(&brand);
                    }
                }
            }
        }
    }
    // 去掉整行开头多余的空白
    let layout_text = layout_text.trim_start().to_string();

    let layout = build_parley_layout(
        &layout_text,
        params,
        font_size,
        None,
        &boxes,
        watermark_params,
        font_ctx,
        layout_ctx,
    );

    let height = (font_size * params.line_spacing as f32)
        .max(layout.height())
        .ceil()
        .max(1.0) as usize;

    Ok(PreparedLine {
        layout,
        slots,
        height,
    })
}

fn build_parley_layout(
    text: &str,
    params: &TextParams,
    font_size: f32,
    max_advance: Option<f32>,
    boxes: &[InlineBox],
    watermark_params: &WatermarkParams,
    font_ctx: &mut FontContext,
    layout_ctx: &mut LayoutContext<peniko::Brush>,
) -> Layout<peniko::Brush> {
    let mut builder = layout_ctx.ranged_builder(font_ctx, text, 1.0, true);
    // 追加一些包含 ℤ (U+2124) 的**无衬线** fallback 字体，
    // 否则 parley 对缺失字形只输出 gid=0 (.notdef)，导致“ℤ”渲染成空白；
    // 同时避免 fallback 到衬线字体，保证 ℤ 跟整体文字一样是“黑体”风格。
    let family = format!(
        "{}, Menlo, Geneva, Arial Unicode MS, Fira Code, sans-serif",
        params.font
    );
    builder.push_default(StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(
        family,
    ))));
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontStyle(if params.italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    }));
    builder.push_default(StyleProperty::FontWeight(if params.bold {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    }));
    builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
        params.line_spacing as f32,
    )));
    builder.push_default(StyleProperty::Brush(text_brush(params, watermark_params)));
    for ibox in boxes {
        builder.push_inline_box(ibox.clone());
    }

    let mut layout = builder.build(text);
    layout.break_all_lines(max_advance);
    layout.align(alignment_for(params.align), AlignmentOptions::default());
    layout
}

fn measure_cap_height(layout: &Layout<peniko::Brush>) -> Option<f32> {
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(gr) = item {
                if let Some(cap) = gr.run().metrics().cap_height {
                    return Some(cap);
                }
            }
        }
    }
    None
}

fn alignment_for(align: TextAlign) -> Alignment {
    match align {
        TextAlign::Left => Alignment::Left,
        TextAlign::Center => Alignment::Center,
        TextAlign::Right => Alignment::Right,
    }
}

fn text_brush(params: &TextParams, watermark_params: &WatermarkParams) -> peniko::Brush {
    let c = text_color(params, watermark_params);
    let color: peniko::Color = peniko::color::Rgba8::from_u8_array(c).into();
    color.into()
}

fn text_color(params: &TextParams, watermark_params: &WatermarkParams) -> [u8; 4] {
    if let Some(rgb) = params.color {
        [rgb[0], rgb[1], rgb[2], 255]
    } else if auto_color(watermark_params) {
        [0, 0, 0, 255]
    } else {
        [255, 255, 255, 255]
    }
}

fn brush_to_rgba(brush: peniko::Brush) -> [u8; 4] {
    match brush {
        peniko::Brush::Solid(c) => c.to_rgba8().to_u8_array(),
        _ => [255, 255, 255, 255],
    }
}

fn render_line_into(
    canvas: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    y_off: i32,
    line: &PreparedLine,
) -> Result<()> {
    let mut scale_ctx = ScaleContext::new();

    for layout_line in line.layout.lines() {
        for item in layout_line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let run = glyph_run.run();
                    let font = run.font();
                    let Some(font_ref) = FontRef::from_index(font.data.data(), font.index as usize)
                    else {
                        continue;
                    };
                    let mut scaler = scale_ctx
                        .builder(font_ref)
                        .size(run.font_size())
                        .hint(true)
                        .build();
                    let color = brush_to_rgba(glyph_run.style().brush.clone());

                    for glyph in glyph_run.positioned_glyphs() {
                        let mut render = Render::new(&[Source::Outline]);
                        render.format(Format::Alpha);
                        if let Some(image) = render.render(&mut scaler, glyph.id as u16) {
                            draw_glyph_mask(
                                canvas, canvas_w, canvas_h, y_off, &image, glyph.x, glyph.y, color,
                            );
                        }
                    }
                }
                PositionedLayoutItem::InlineBox(inline_box) => {
                    if inline_box.kind == InlineBoxKind::InFlow {
                        if let Some(slot) = line.slots.get(inline_box.id as usize) {
                            draw_logo(
                                canvas,
                                canvas_w,
                                canvas_h,
                                y_off,
                                inline_box.x as i32,
                                inline_box.y as i32,
                                &slot.image,
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn draw_glyph_mask(
    canvas: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    y_off: i32,
    image: &Image,
    gx: f32,
    gy: f32,
    color: [u8; 4],
) {
    if image.content != Content::Mask {
        return;
    }
    let pw = image.placement.width as i32;
    let ph = image.placement.height as i32;
    let px = gx as i32 + image.placement.left;
    let py = gy as i32 - image.placement.top + y_off;

    for row in 0..ph {
        let dy = py + row;
        if dy < 0 || dy >= canvas_h {
            continue;
        }
        for col in 0..pw {
            let dx = px + col;
            if dx < 0 || dx >= canvas_w {
                continue;
            }
            let coverage = image.data[(row * pw + col) as usize];
            if coverage == 0 {
                continue;
            }
            blend_pixel(
                canvas, canvas_w, dx, dy, color[0], color[1], color[2], coverage,
            );
        }
    }
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
    let (logo_w, logo_h) = (logo.get_width(), logo.get_height());
    let bands = logo.get_bands();
    if bands != 4 {
        return Err(VipsError::OperationError("logo image must be RGBA"));
    }
    let data = logo.image_write_to_memory();
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
    let img = ops::cast(&img, ops::BandFormat::Uchar)?;
    let bands = img.get_bands();
    let rgba = match bands {
        4 => img,
        3 => ops::addalpha(&img)?,
        2 => {
            let grey = ops::extract_band(&img, 0)?;
            let alpha = ops::extract_band(&img, 1)?;
            let g2 = ops::copy(&grey)?;
            let g3 = ops::copy(&grey)?;
            let rgb = ops::bandjoin(&mut [grey, g2, g3])?;
            ops::bandjoin(&mut [rgb, alpha])?
        }
        1 => {
            let g1 = img;
            let g2 = ops::copy(&g1)?;
            let g3 = ops::copy(&g1)?;
            let rgb = ops::bandjoin(&mut [g1, g2, g3])?;
            let alpha = VipsImage::new_from_image(&rgb, &[255.0])?;
            ops::bandjoin(&mut [rgb, alpha])?
        }
        _ => {
            return Err(VipsError::OperationError(
                "unsupported logo bands, expected 1..=4",
            ));
        }
    };

    let w = rgba.get_width();
    let h = rgba.get_height();
    ops::copy_with_opts(
        &rgba,
        &ops::CopyOptions {
            width: w,
            height: h,
            bands: 4,
            interpretation: ops::Interpretation::Srgb,
            ..Default::default()
        },
    )
}

fn scale_logo(img: VipsImage, target_w: i32, target_h: i32) -> Result<VipsImage> {
    let img = to_rgba(img)?;
    let (w, h) = (img.get_width(), img.get_height());
    if (w, h) != (target_w, target_h) {
        let scale_x = target_w as f64 / w as f64;
        let scale_y = target_h as f64 / h as f64;
        return ops::resize_with_opts(
            &img,
            scale_x,
            &ops::ResizeOptions {
                vscale: scale_y,
                ..Default::default()
            },
        );
    }
    Ok(img)
}

/// 根据给定的模板生成文字。
///
/// 缺失的字段不会被保留成 `{xxx}` 字面量，而是直接丢弃，
pub fn render_exif_template(template: &str, exif: &ExifInfo, time_format: &str) -> String {
    let re = Regex::new(r"\{([^{}]+)\}").unwrap();
    let mut out = String::new();
    let mut last = 0usize;

    for caps in re.captures_iter(template) {
        let whole = caps.get(0).unwrap();
        if whole.start() > last {
            out.push_str(&template[last..whole.start()]);
        }
        let key = &caps[1];
        if let Some(value) = resolve_exif_key_name(key, exif, time_format) {
            if !value.is_empty() {
                out.push_str(&value);
            }
        }
        last = whole.end();
    }
    if last < template.len() {
        out.push_str(&template[last..]);
    }
    out
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
        // Logo 由 render_text 的 InlineBox 处理，这里不替换成文本
        "Logo" => None,
        _ => None,
    }
}

fn format_created_time(value: &ExifDateTime, time_format: &str) -> String {
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
            .replace('Z', "ℤ")
            .replace('z', "ℤ")
            .split('_')
            .collect::<Vec<_>>()
            .split_last()
            .map(|(last, rest)| {
                if let Ok(num) = last.parse::<i32>() {
                    format!("{} {}", rest.join(" "), crate::helper::to_roman(num))
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

fn find_make_logo(make: &str, watermark_params: &WatermarkParams) -> Option<VipsImage> {
    let make = make.replace("CORPORATION", "").trim().to_lowercase();
    let suffix = if auto_color(watermark_params) {
        "b"
    } else {
        "w"
    };
    let path = format!("./static/logo/{}-{}.svg", make, suffix);
    let svg = std::fs::read_to_string(path).ok()?;
    ops::svgload_buffer(svg.as_bytes()).ok()
}
