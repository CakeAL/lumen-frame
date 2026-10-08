//! 预览与导出共用的分层排版；后端决定图层是 libvips 图像还是已缓存的界面位图。

use anyhow::Result;

use crate::{
    render::{canvas, image},
    watermark::{Placement, TextAlign, TextGroup, WatermarkParams},
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
    fn background(&mut self, size: (i32, i32)) -> Result<Self::Image>;
    fn photo(&mut self) -> Result<Self::Image>;
    /// 阴影坐标相对于照片左上角；与画布大小及照片放置位置无关。
    fn shadow(&mut self) -> Result<Option<PositionedLayer<Self::Image>>>;
}

pub(crate) fn prepare<R: LayerRenderer>(
    renderer: &mut R,
    (img_w, img_h): (i32, i32),
    params: &WatermarkParams,
    groups: &[TextGroup],
) -> Result<WatermarkLayers<R::Image>> {
    let mut texts = Vec::new();
    let mut dimensions = vec![None; groups.len()];
    let mut content_bounds = vec![None; groups.len()];
    let has_attachments = groups.iter().any(|group| group.attachment.is_some());
    for (group_ix, group) in groups.iter().enumerate() {
        if let Some(image) = renderer.text(group)? {
            dimensions[group_ix] = Some(R::dimensions(&image));
            let (w, h) = R::dimensions(&image);
            content_bounds[group_ix] = Some(if has_attachments {
                R::content_bounds(&image)?
            } else {
                (0, 0, w, h)
            });
            texts.push((group_ix, group, image));
        }
    }
    let mut origins = vec![None; groups.len()];
    for (ix, _, _) in &texts {
        local_origin(
            *ix,
            groups,
            &dimensions,
            &content_bounds,
            img_h,
            &mut origins,
            &mut vec![false; groups.len()],
        )?;
    }
    // 每棵相邻排版树作为一个整体参与边框厚度与对齐计算。
    let mut bounds = vec![None::<(i32, i32, i32, i32)>; groups.len()];
    for (ix, _, _) in &texts {
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
        let side = match groups[root].position {
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
    layers.push(PositionedLayer {
        image: photo,
        x: img_x,
        y: img_y,
        group_ix: None,
    });
    let mut cluster_origins = vec![(0, 0); groups.len()];
    for (root, bounds) in bounds.iter().enumerate() {
        let Some((left, top, right, bottom)) = *bounds else {
            continue;
        };
        let group = &groups[root];
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
            Placement::Up => (aligned_x, (img_y - text_h) / 2),
            Placement::Bottom => (
                aligned_x,
                img_y + img_h + (canvas_h - img_y - img_h - text_h) / 2,
            ),
            Placement::Left => ((img_x - text_w) / 2, aligned_y),
            Placement::Right => (
                img_x + img_w + (canvas_w - img_x - img_w - text_w) / 2,
                aligned_y,
            ),
            Placement::Center => (aligned_x, img_y + (img_h - text_h) / 2),
        };
        cluster_origins[root] = (x - left, y - top);
    }
    for (group_ix, _, image) in texts {
        let (root, x, y) = origins[group_ix].unwrap();
        let origin = cluster_origins[root];
        layers.push(PositionedLayer {
            image,
            x: origin.0 + x,
            y: origin.1 + y,
            group_ix: Some(group_ix),
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
    groups: &[TextGroup],
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
