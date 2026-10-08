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
    let mut thickness = [0; 4];
    for (group_ix, group) in groups.iter().enumerate() {
        if let Some(image) = renderer.text(group)? {
            let (w, h) = R::dimensions(&image);
            let side = match group.position {
                Placement::Up => Some((0, h)),
                Placement::Right => Some((1, w)),
                Placement::Bottom => Some((2, h)),
                Placement::Left => Some((3, w)),
                Placement::Center => None,
            };
            if let Some((side, value)) = side {
                thickness[side] = thickness[side].max(value);
            }
            texts.push((group_ix, group, image));
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
    for (group_ix, group, image) in texts {
        let (text_w, text_h) = R::dimensions(&image);
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
        layers.push(PositionedLayer {
            image,
            x,
            y,
            group_ix: Some(group_ix),
        });
    }
    Ok(WatermarkLayers {
        size,
        photo_origin: (img_x, img_y),
        layers,
    })
}
