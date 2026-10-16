//! 预览与导出共用的分层排版；后端决定图层是 libvips 图像还是已缓存的界面位图。

use anyhow::Result;

use crate::{
    render::{canvas, image},
    watermark::{GroupAttachment, Placement, TextAlign, TextGroup, WatermarkParams},
};

pub(crate) struct PositionedLayer<I> {
    pub image: I,
    pub x: i32,
    pub y: i32,
    pub group_ix: Option<usize>,
}

pub(crate) struct WatermarkLayers<I> {
    pub size: (i32, i32),
    pub photo_origin: (i32, i32),
    pub layers: Vec<PositionedLayer<I>>,
}

pub(crate) trait LayerRenderer {
    type Image;

    fn dimensions(image: &Self::Image) -> (i32, i32);
    fn content_bounds(image: &Self::Image) -> Result<(i32, i32, i32, i32)>;
    fn text(&mut self, group: &TextGroup) -> Result<Option<Self::Image>>;
    fn palette(&mut self) -> Result<Option<Self::Image>>;
    fn background(&mut self, size: (i32, i32)) -> Result<Self::Image>;
    fn photo(&mut self) -> Result<Self::Image>;
    /// 阴影坐标相对于照片左上角；与画布大小及照片放置位置无关。
    fn shadow(&mut self) -> Result<Option<PositionedLayer<Self::Image>>>;
    fn border(&mut self) -> Result<Option<PositionedLayer<Self::Image>>>;
}

/// 内容类型不参与定位：文字和色卡共用相邻树、对齐和留白计算。
struct LayoutPlacement {
    position: Placement,
    align: TextAlign,
    attachment: Option<GroupAttachment>,
}

pub(crate) fn prepare<R: LayerRenderer>(
    renderer: &mut R,
    (img_w, img_h): (i32, i32),
    params: &WatermarkParams,
    groups: &[TextGroup],
) -> Result<WatermarkLayers<R::Image>> {
    let palette_ix = groups.len();
    let mut placements = groups
        .iter()
        .map(|group| LayoutPlacement {
            position: group.position,
            align: group.align,
            attachment: group.attachment.clone().filter(|a| a.target < groups.len()),
        })
        .collect::<Vec<_>>();
    let card = &params.palette_card;
    placements.push(LayoutPlacement {
        position: card.position(),
        align: card.align(),
        attachment: card
            .attachment()
            .cloned()
            .filter(|a| a.target < groups.len()),
    });
    let mut texts = Vec::new();
    let mut dimensions = vec![None; placements.len()];
    let mut content_bounds = vec![None; placements.len()];
    let has_attachments = placements.iter().any(|group| group.attachment.is_some());
    for (group_ix, group) in groups.iter().enumerate() {
        if let Some(image) = renderer.text(group)? {
            let (w, h) = R::dimensions(&image);
            dimensions[group_ix] = Some((w, h));
            content_bounds[group_ix] = Some(if has_attachments {
                R::content_bounds(&image)?
            } else {
                (0, 0, w, h)
            });
            texts.push((group_ix, image));
        }
    }
    let mut palette_offset = (0, 0);
    if let Some(image) = renderer.palette()? {
        let (w, h) = R::dimensions(&image);
        // 与文字组相同：只有靠边对齐时，才在相应方向添加透明留白；依附时由组间距控制。
        let pad = if card.attachment().is_none() && card.align() != TextAlign::Center {
            (card.padding() * img_h as f64).round() as i32
        } else {
            0
        };
        let vertical = matches!(card.position(), Placement::Left | Placement::Right);
        if card.align() == TextAlign::Left {
            palette_offset = if vertical { (0, pad) } else { (pad, 0) };
        }
        dimensions[palette_ix] = Some(if vertical { (w, h + pad) } else { (w + pad, h) });
        content_bounds[palette_ix] = Some((palette_offset.0, palette_offset.1, w, h));
        texts.push((palette_ix, image));
    }
    let mut origins = vec![None; placements.len()];
    for (ix, _) in &texts {
        local_origin(
            *ix,
            &placements,
            &dimensions,
            &content_bounds,
            img_h,
            &mut origins,
            &mut vec![false; placements.len()],
        )?;
    }
    // 每棵相邻排版树作为一个整体参与边框厚度与对齐计算。
    let mut bounds = vec![None::<(i32, i32, i32, i32)>; placements.len()];
    for (ix, _) in &texts {
        let (root, x, y) = origins[*ix].unwrap();
        let (w, h) = dimensions[*ix].unwrap();
        let old = bounds[root].unwrap_or((x, y, x + w, y + h));
        bounds[root] = Some((
            old.0.min(x),
            old.1.min(y),
            old.2.max(x + w),
            old.3.max(y + h),
        ));
    }
    let mut thickness = [0; 4];
    for (root, bounds) in bounds.iter().enumerate() {
        let Some((left, top, right, bottom)) = bounds else {
            continue;
        };
        let side = match placements[root].position {
            Placement::Up => Some((0, bottom - top)),
            Placement::Right => Some((1, right - left)),
            Placement::Bottom => Some((2, bottom - top)),
            Placement::Left => Some((3, right - left)),
            Placement::Center => None,
        };
        if let Some((side, value)) = side {
            thickness[side] = thickness[side].max(value);
        }
    }
    let mut margin = canvas::Margin::cal_margin(img_w, img_h, params);
    for (side, value) in [
        Placement::Up,
        Placement::Right,
        Placement::Bottom,
        Placement::Left,
    ]
    .into_iter()
    .zip(thickness)
    {
        margin.include_text_thickness(side, value);
    }
    // 色卡所在的相邻树整体可能超出照片，按根组对齐方向扩展，保留首尾内容。
    if let Some((root, _, _)) = origins[palette_ix] {
        let (left, top, right, bottom) = bounds[root].unwrap();
        let placement = &placements[root];
        let mut fit = |before, after, extent: i32, available: i32, align| {
            let overflow = (extent - available).max(0);
            let (leading, trailing) = match align {
                TextAlign::Left => (0, overflow),
                TextAlign::Center => ((overflow + 1) / 2, (overflow + 1) / 2),
                TextAlign::Right => (overflow, 0),
            };
            margin.include_text_thickness(before, leading);
            margin.include_text_thickness(after, trailing);
        };
        match placement.position {
            Placement::Up | Placement::Bottom => fit(
                Placement::Left,
                Placement::Right,
                right - left,
                img_w,
                placement.align,
            ),
            Placement::Left | Placement::Right => fit(
                Placement::Up,
                Placement::Bottom,
                bottom - top,
                img_h,
                placement.align,
            ),
            Placement::Center => {
                fit(
                    Placement::Left,
                    Placement::Right,
                    right - left,
                    img_w,
                    placement.align,
                );
                fit(
                    Placement::Up,
                    Placement::Bottom,
                    bottom - top,
                    img_h,
                    TextAlign::Center,
                );
            }
        }
    }
    let size = canvas::cal_size(&margin, img_w, img_h, params);
    let (canvas_w, canvas_h) = size;
    let (img_x, img_y) = image::cal_coordinates(&margin, canvas_w, canvas_h, img_w, img_h, params);
    let mut layers = vec![PositionedLayer {
        image: renderer.background(size)?,
        x: 0,
        y: 0,
        group_ix: None,
    }];
    let photo = renderer.photo()?;
    if let Some(mut shadow) = renderer.shadow()? {
        shadow.x += img_x;
        shadow.y += img_y;
        layers.push(shadow);
    }
    if let Some(mut border) = renderer.border()? {
        border.x += img_x;
        border.y += img_y;
        layers.push(border);
    }
    layers.push(PositionedLayer {
        image: photo,
        x: img_x,
        y: img_y,
        group_ix: None,
    });
    let frame = params.photo_border.pixels(img_h);
    let mut cluster_origins = vec![(0, 0); placements.len()];
    for (root, bounds) in bounds.iter().enumerate() {
        let Some((left, top, right, bottom)) = *bounds else {
            continue;
        };
        let group = &placements[root];
        let (text_w, text_h) = (right - left, bottom - top);
        let aligned_x = match group.align {
            TextAlign::Left => img_x,
            TextAlign::Center => img_x + (img_w - text_w) / 2,
            TextAlign::Right => img_x + img_w - text_w,
        };
        let aligned_y = match group.align {
            TextAlign::Left => img_y,
            TextAlign::Center => img_y + (img_h - text_h) / 2,
            TextAlign::Right => img_y + img_h - text_h,
        };
        let (x, y) = match group.position {
            Placement::Up => (aligned_x, (img_y - frame - text_h) / 2),
            Placement::Bottom => (
                aligned_x,
                img_y + img_h + frame + (canvas_h - img_y - img_h - frame - text_h) / 2,
            ),
            Placement::Left => ((img_x - frame - text_w) / 2, aligned_y),
            Placement::Right => (
                img_x + img_w + frame + (canvas_w - img_x - img_w - frame - text_w) / 2,
                aligned_y,
            ),
            Placement::Center => (aligned_x, img_y + (img_h - text_h) / 2),
        };
        cluster_origins[root] = (x - left, y - top);
    }
    for (group_ix, image) in texts {
        let (root, x, y) = origins[group_ix].unwrap();
        let mut origin = cluster_origins[root];
        if group_ix == palette_ix {
            origin.0 += palette_offset.0;
            origin.1 += palette_offset.1;
        }
        layers.push(PositionedLayer {
            image,
            x: origin.0 + x,
            y: origin.1 + y,
            group_ix: (group_ix < palette_ix).then_some(group_ix),
        });
    }
    Ok(WatermarkLayers {
        size,
        photo_origin: (img_x, img_y),
        layers,
    })
}

/// 局部坐标与目标组中心对齐；内容为空或引用已失效时恢复独立位置。
fn local_origin(
    ix: usize,
    groups: &[LayoutPlacement],
    dimensions: &[Option<(i32, i32)>],
    content_bounds: &[Option<(i32, i32, i32, i32)>],
    img_h: i32,
    origins: &mut [Option<(usize, i32, i32)>],
    visiting: &mut [bool],
) -> Result<(usize, i32, i32)> {
    if let Some(origin) = origins[ix] {
        return Ok(origin);
    }
    anyhow::ensure!(
        !visiting[ix],
        "文字组相邻关系不能形成循环，请更改目标文字组"
    );
    visiting[ix] = true;
    let attachment = groups[ix]
        .attachment
        .as_ref()
        .filter(|a| a.target != ix && dimensions.get(a.target).is_some_and(Option::is_some));
    let origin = if let Some(a) = attachment {
        let (root, x, y) = local_origin(
            a.target,
            groups,
            dimensions,
            content_bounds,
            img_h,
            origins,
            visiting,
        )?;
        let (tx, ty, tw, th) = content_bounds[a.target].unwrap();
        let (cx, cy, w, h) = content_bounds[ix].unwrap();
        let gap = (a.gap.max(0.0) * img_h as f64).round() as i32;
        let (dx, dy) = match a.side {
            Placement::Left => (tx - cx - w - gap, ty - cy + (th - h) / 2),
            Placement::Right => (tx - cx + tw + gap, ty - cy + (th - h) / 2),
            Placement::Up => (tx - cx + (tw - w) / 2, ty - cy - h - gap),
            Placement::Bottom => (tx - cx + (tw - w) / 2, ty - cy + th + gap),
            Placement::Center => (tx - cx + (tw - w) / 2, ty - cy + (th - h) / 2),
        };
        (root, x + dx, y + dy)
    } else {
        (ix, 0, 0)
    };
    visiting[ix] = false;
    origins[ix] = Some(origin);
    Ok(origin)
}
