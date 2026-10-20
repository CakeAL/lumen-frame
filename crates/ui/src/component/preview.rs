//! 中间上方的预览面板。
//!
//! 面板本身只负责呈现，渲染节奏由 [`WatermarkPreview`] 这个实体决定：参数每变一次就
//! 请求一次，请求在实体里合并、防抖，再到后台线程重算。这样拖动滑块时不会每帧都启动
//! 一次图层更新，也不会让已经过期的结果覆盖新结果。

use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, Size, StyledExt as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::*;
#[cfg(test)]
use gpui_kit::test::TestSupportExt as _;
use gpui_kit::{
    AvailableSpace, Bounds, Context, ElementId, EntityId, FontWeight, ObjectFit, Pixels,
    RenderImage, SharedString, Task, Window, canvas, div, point, px, size,
};

use super::super::AppView;
use super::field::{description, rgb_to_hsla};
use crate::backend::render::text::TextGroupRegion;
use crate::backend::{
    watermark::{PhotoAnchor, Placement, TextAlign},
    workspace::PhotoId,
};
use crate::image::{LayeredPreview, PreviewJob, PreviewLayerCache};

/// 参数连续变化时先攒一会儿再算。
///
/// 这个值决定「跟手」和「算得少」的平衡：太小则拖动滑块时排满后台任务，太大则有明显
/// 的延迟感。
const DEBOUNCE: Duration = Duration::from_millis(80);

#[derive(Clone, Copy)]
enum PreviewEditTarget {
    Text(u64),
    Palette,
}

#[derive(Clone, Copy)]
struct PreviewPlacement {
    outside: (Placement, TextAlign),
    inside: Option<PhotoAnchor>,
}

#[derive(Clone)]
struct DraggedPreviewItem {
    owner: EntityId,
    photo: PhotoId,
    generation: u64,
    target: PreviewEditTarget,
    label: SharedString,
}

impl Render for DraggedPreviewItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(crate::theme::preview_edit_color(cx))
            .shadow_md()
            .child(self.label.clone())
    }
}

/// 分层预览的状态。
///
/// 重算时保留上一张图（[`PreviewState::Rendering`] 的 `previous`），是为了让拖动滑块
/// 的过程中画面连续变化，而不是每次重算都闪一下空白。
pub enum PreviewState {
    /// 队列里没有可预览的照片。
    Empty,
    Rendering {
        previous: Option<Arc<LayeredPreview>>,
    },
    Ready(Arc<LayeredPreview>),
    Failed(SharedString),
}

impl PreviewState {
    /// 按需读取完整位图；窗口绘制使用 layers，不触发 CPU 压平。
    pub(in crate::app) fn image(&self) -> Option<&Arc<RenderImage>> {
        self.layers().map(|layers| layers.image())
    }

    fn layers(&self) -> Option<&Arc<LayeredPreview>> {
        match self {
            Self::Rendering { previous } => previous.as_ref(),
            Self::Ready(layers) => Some(layers),
            Self::Empty | Self::Failed(_) => None,
        }
    }
}

/// 预览的渲染节奏与结果。
pub struct WatermarkPreview {
    state: PreviewState,
    /// 每次请求自增。渲染完成时对不上，就说明这份结果已经过期，直接丢弃。
    generation: u64,
    /// 还没交给后台线程的最新一次请求。
    pending: Option<(PreviewJob, Vec<u64>)>,
    /// 仅最新请求的位图可以编辑；旧图仍显示时不保留它的交互区域。
    text_regions: Vec<(u64, TextGroupRegion)>,
    palette_region: Option<TextGroupRegion>,
    worker: Option<Task<()>>,
    cache: Option<PreviewLayerCache>,
}

impl WatermarkPreview {
    pub fn new() -> Self {
        Self {
            state: PreviewState::Empty,
            generation: 0,
            pending: None,
            text_regions: Vec::new(),
            palette_region: None,
            worker: None,
            cache: Some(PreviewLayerCache::default()),
        }
    }

    pub fn state(&self) -> &PreviewState {
        &self.state
    }

    /// 请求一次预览。同一轮防抖内的多次请求会合并成最后一次。
    pub fn request(&mut self, job: PreviewJob, group_ids: Vec<u64>, cx: &mut Context<Self>) {
        self.pending = Some((job, group_ids));
        self.generation = self.generation.wrapping_add(1);
        self.text_regions.clear();
        self.palette_region = None;
        cx.notify();

        // 已经有循环在跑时什么都不用做：它会在下一轮取走刚存下的请求。
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_ready())
        {
            return;
        }

        self.worker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(DEBOUNCE).await;

                let Some((job, group_ids)) = this
                    .update(cx, |this, _| this.pending.take())
                    .ok()
                    .flatten()
                else {
                    break;
                };
                let Ok(generation) = this.update(cx, |this, _| this.generation) else {
                    break;
                };

                let started = this.update(cx, |this, cx| {
                    this.state = PreviewState::Rendering {
                        previous: this.state.layers().cloned(),
                    };
                    cx.notify();
                });
                if started.is_err() {
                    break;
                }

                let Ok(mut cache) =
                    this.update(cx, |this, _| this.cache.take().unwrap_or_default())
                else {
                    break;
                };
                let (cache, rendered) = cx
                    .background_spawn(async move {
                        let rendered = cache.render(&job);
                        (cache, rendered)
                    })
                    .await;

                let applied = this.update(cx, |this, cx| {
                    // 缓存仅是可重用位图；各层键会核对照片、文件版本与参数。
                    // 清空队列时不恢复正在后台生成的旧照片缓存。
                    if !matches!(this.state, PreviewState::Empty) {
                        this.cache = Some(cache);
                    }
                    // 期间用户又改了参数：这份结果对应的已经不是界面上的状态了。
                    if this.generation != generation {
                        return;
                    }
                    this.state = match rendered {
                        Ok(rendered) => {
                            this.palette_region = rendered.palette_region;
                            this.text_regions = rendered
                                .text_regions
                                .iter()
                                .copied()
                                .filter_map(|region| {
                                    group_ids.get(region.group_ix).map(|id| (*id, region))
                                })
                                .collect();
                            PreviewState::Ready(rendered)
                        }
                        Err(error) => PreviewState::Failed(format!("{error:#}").into()),
                    };
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        }));
    }

    /// 当前没有可预览的照片（队列为空，或选中的照片被移除了）。
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        self.cache = Some(PreviewLayerCache::default());
        self.text_regions.clear();
        self.palette_region = None;
        self.generation = self.generation.wrapping_add(1);
        if !matches!(self.state, PreviewState::Empty) {
            self.state = PreviewState::Empty;
        }
        cx.notify();
    }
}

impl AppView {
    pub(in crate::app) fn render_preview_pane(&self, cx: &Context<Self>) -> impl IntoElement {
        let title = self.selected_photo().map(|photo| {
            photo
                .path()
                .file_name()
                .map(|name| SharedString::from(name.to_string_lossy().into_owned()))
                .unwrap_or_else(|| SharedString::from(photo.path().to_string_lossy().into_owned()))
        });

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.render_preview_header(title, cx))
            .child(self.render_preview_canvas(cx))
    }

    fn render_preview_header(
        &self,
        title: Option<SharedString>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .justify_between()
            .gap_3()
            .px_4()
            .h_12()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().foreground)
                    .child(title.unwrap_or_else(|| "预览".into())),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .when(self.selected_photo_id().is_some(), |this| {
                        this.child(description("点击编辑，拖动调整位置，右键添加"))
                    })
                    .child(self.render_preview_status(cx))
                    .child(
                        Button::new("preview-exif")
                            .icon(IconName::Info)
                            .label("EXIF 信息…")
                            .outline()
                            .small()
                            .disabled(self.selected_photo_id().is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_exif_editor(window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new("preview-gps")
                            .label("更改GPS信息")
                            .outline()
                            .small()
                            .disabled(self.selected_photo_id().is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_location_picker(window, cx)
                            })),
                    ),
            )
    }

    /// 只在预览不是最新的时候说话：正在算就说正在算，失败就说失败。
    fn render_preview_status(&self, cx: &Context<Self>) -> impl IntoElement {
        let state = self.preview.read(cx).state();
        match state {
            PreviewState::Rendering { .. } => h_flex()
                .flex_shrink_0()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Spinner::new().small())
                .child(description("正在更新预览"))
                .into_any_element(),
            PreviewState::Failed(message) => h_flex()
                .min_w_0()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(Icon::new(IconName::TriangleAlert).small())
                .child(div().truncate().child(message.clone()))
                .into_any_element(),
            PreviewState::Empty | PreviewState::Ready(_) => div().into_any_element(),
        }
    }

    fn render_preview_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let state = self.preview.read(cx).state();
        let content = match state {
            PreviewState::Empty => div()
                .v_flex()
                .items_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Frame).with_size(Size::Large))
                .child(description(
                    "把照片或文件夹拖到这里，或用下方的添加按钮选择",
                ))
                .into_any_element(),
            PreviewState::Failed(message) => div()
                .v_flex()
                .items_center()
                .gap_3()
                .max_w_96()
                .text_color(cx.theme().danger)
                .child(Icon::new(IconName::TriangleAlert).with_size(Size::Large))
                .child(div().text_sm().child(message.clone()))
                .into_any_element(),
            PreviewState::Ready(image) => self.render_editable_bitmap(image, cx).into_any_element(),
            PreviewState::Rendering { previous } => match previous {
                Some(image) => render_bitmap(image).into_any_element(),
                None => div()
                    .v_flex()
                    .items_center()
                    .gap_3()
                    .text_color(cx.theme().muted_foreground)
                    .child(Spinner::new().with_size(Size::Large))
                    .child(div().text_sm().child("正在生成预览…"))
                    .into_any_element(),
            },
        };

        v_flex()
            .size_full()
            .min_h_0()
            .items_center()
            .justify_center()
            .bg(rgb_to_hsla(self.preview_background))
            .p_6()
            .child(content)
    }

    fn render_editable_bitmap(
        &self,
        image: &Arc<LayeredPreview>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let preview = self.preview.read(cx);
        let generation = preview.generation;
        let mut regions = preview
            .text_regions
            .iter()
            .map(|(id, region)| (PreviewEditTarget::Text(*id), *region))
            .collect::<Vec<_>>();
        if let Some(region) = preview.palette_region {
            // 色卡最后绘制，命中区域也位于文字组之上。
            regions.push((PreviewEditTarget::Palette, region));
        }
        let image_size = image.size();
        let photo_region = image.photo_region();
        let app = cx.entity().downgrade();
        let owner = cx.entity_id();
        let photo = self
            .selected_photo_id()
            .expect("可编辑预览必须属于当前照片");
        let drop_app = app.clone();
        let menu_app = app.clone();
        let rotation = self.backend.params().rotation;
        let fitted_bounds = Rc::new(Cell::new(None));
        let drop_bounds = fitted_bounds.clone();
        let menu_bounds = fitted_bounds.clone();

        div()
            .id("preview-bitmap")
            .map(|this| {
                #[cfg(test)]
                let this = this.test_support();
                this
            })
            .relative()
            .size_full()
            .child(render_bitmap(image))
            .child(
                // 在布局完成后使用与各图层完全相同的 contain 变换，
                // 为每个可编辑图层摆放标准 Button，复用键盘、焦点、提示和无障碍语义。
                canvas(
                    move |bounds, window, cx| {
                        let fitted = ObjectFit::Contain.get_bounds(bounds, image_size);
                        fitted_bounds.set(Some(fitted));
                        let mut elements = Vec::with_capacity(regions.len());
                        for (target, region) in regions {
                            let rect = preview_region_bounds(fitted, image_size, region);
                            let app = app.clone();
                            let (id, label): (ElementId, String) = match target {
                                PreviewEditTarget::Text(id) => (
                                    ("preview-text-group", id).into(),
                                    format!("编辑文字组 {}…", region.group_ix + 1),
                                ),
                                PreviewEditTarget::Palette => {
                                    ("preview-palette-card".into(), "编辑照片色卡…".into())
                                }
                            };
                            let primary = crate::theme::preview_edit_color(cx);
                            let dragged = DraggedPreviewItem {
                                owner,
                                photo,
                                generation,
                                target,
                                label: label.clone().into(),
                            };
                            let mut element = Button::new(id)
                                .custom(
                                    ButtonCustomVariant::new(cx)
                                        .foreground(primary)
                                        .hover(primary.opacity(0.22))
                                        .active(primary.opacity(0.3)),
                                )
                                .p_0()
                                .w(rect.size.width)
                                .h(rect.size.height)
                                .accessibility_label(label.clone())
                                .tooltip(label)
                                .on_click(move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        // 事件可能来自上一帧；删除文字组或切换照片后不能
                                        // 让旧的命中区域打开另一个配置的编辑窗口。
                                        let preview = this.preview.read(cx);
                                        let current = preview.generation == generation
                                            && match target {
                                                PreviewEditTarget::Text(id) => preview
                                                    .text_regions
                                                    .iter()
                                                    .any(|(group_id, _)| *group_id == id),
                                                PreviewEditTarget::Palette => {
                                                    preview.palette_region.is_some()
                                                        && this
                                                            .backend
                                                            .params()
                                                            .palette_card
                                                            .enabled()
                                                }
                                            };
                                        if current {
                                            match target {
                                                PreviewEditTarget::Text(id) => {
                                                    this.open_text_group_editor(id, window, cx)
                                                }
                                                PreviewEditTarget::Palette => {
                                                    this.open_palette_card(window, cx)
                                                }
                                            }
                                            // 重叠图层只打开最上层的编辑器。
                                            cx.stop_propagation();
                                        }
                                    })
                                    .ok();
                                })
                                .map(|mut button| {
                                    button
                                        .interactivity()
                                        .on_drag(dragged, |dragged, _, _, cx| {
                                            cx.new(|_| dragged.clone())
                                        });
                                    button
                                })
                                .into_any_element();
                            element.prepaint_as_root(
                                rect.origin,
                                rect.size.map(AvailableSpace::Definite),
                                window,
                                cx,
                            );
                            elements.push(element);
                        }
                        elements
                    },
                    |_, mut elements, window, cx| {
                        for element in &mut elements {
                            element.paint(window, cx);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .on_drop(move |dragged: &DraggedPreviewItem, window, cx| {
                let Some(fitted) = drop_bounds.get() else {
                    return;
                };
                let placement = drop_placement(
                    fitted,
                    window.mouse_position(),
                    rotation,
                    preview_region_bounds(fitted, image_size, photo_region),
                );
                let _ = drop_app.update(cx, |this, cx| {
                    if dragged.owner == cx.entity_id()
                        && this.selected_photo_id() == Some(dragged.photo)
                        && this.preview.read(cx).generation == dragged.generation
                    {
                        this.move_preview_item(dragged.target, placement, cx);
                        cx.stop_propagation();
                    }
                });
            })
            .context_menu(move |menu, window, cx| {
                let (Some(fitted), Some(view)) = (menu_bounds.get(), menu_app.upgrade()) else {
                    return menu;
                };
                let placement = drop_placement(
                    fitted,
                    window.mouse_position(),
                    rotation,
                    preview_region_bounds(fitted, image_size, photo_region),
                );
                let has_palette = view.read(cx).backend.params().palette_card.enabled();
                let text_app = view.downgrade();
                let palette_app = view.downgrade();
                let delete_app = view.downgrade();
                let hovered_group = view
                    .read(cx)
                    .preview
                    .read(cx)
                    .text_regions
                    .iter()
                    .rev()
                    .find(|(_, region)| {
                        preview_region_bounds(fitted, image_size, *region)
                            .contains(&window.mouse_position())
                    })
                    .map(|(id, _)| *id);
                menu.item(
                    PopupMenuItem::new("添加文字组…").on_click(move |_, window, cx| {
                        let _ = text_app.update(cx, |this, cx| {
                            if this.selected_photo_id() != Some(photo)
                                || this.preview.read(cx).generation != generation
                            {
                                return;
                            }
                            this.add_text_group(window, cx);
                            if let Some(group) = this.text_groups.last() {
                                this.move_preview_item(
                                    PreviewEditTarget::Text(group.id),
                                    placement,
                                    cx,
                                );
                            }
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new(if has_palette {
                        "编辑色卡…"
                    } else {
                        "添加色卡…"
                    })
                    .on_click(move |_, window, cx| {
                        let _ = palette_app.update(cx, |this, cx| {
                            if this.selected_photo_id() != Some(photo)
                                || this.preview.read(cx).generation != generation
                            {
                                return;
                            }
                            if !this.backend.params().palette_card.enabled() {
                                this.backend.params_mut().palette_card.set_enabled(true);
                                this.move_preview_item(PreviewEditTarget::Palette, placement, cx);
                            }
                            this.open_palette_card(window, cx);
                        });
                    }),
                )
                .when_some(hovered_group, |menu, id| {
                    menu.separator()
                        .item(PopupMenuItem::new("删除文字组").on_click(move |_, _, cx| {
                            let _ = delete_app.update(cx, |this, cx| {
                                if this.selected_photo_id() == Some(photo)
                                    && this.preview.read(cx).generation == generation
                                {
                                    this.remove_text_group(id, cx);
                                }
                            });
                        }))
                })
            })
    }

    fn move_preview_item(
        &mut self,
        target: PreviewEditTarget,
        placement: PreviewPlacement,
        cx: &mut Context<Self>,
    ) {
        let (position, align) = placement.outside;
        match target {
            PreviewEditTarget::Text(id) => {
                let Some(group) = self.text_groups.iter_mut().find(|group| group.id == id) else {
                    return;
                };
                group.attachment_target = None;
                group.position = position;
                group.align = align;
                group.photo_anchor = placement.inside;
            }
            PreviewEditTarget::Palette => {
                let card = &mut self.backend.params_mut().palette_card;
                card.set_attachment(None);
                card.set_position(position);
                card.set_align(align);
            }
        }
        self.refresh_preview(cx);
        cx.notify();
    }
}

fn drop_placement(
    fitted: Bounds<Pixels>,
    point: gpui_kit::Point<Pixels>,
    rotation: crate::backend::rotation::Rotation,
    photo: Bounds<Pixels>,
) -> PreviewPlacement {
    let outside = Placement::from_normalized_point(
        f64::from((point.x - fitted.origin.x).as_f32() / fitted.size.width.as_f32().max(1.0)),
        f64::from((point.y - fitted.origin.y).as_f32() / fitted.size.height.as_f32().max(1.0)),
        rotation,
    );
    let inside = photo.contains(&point).then(|| {
        PhotoAnchor::from_normalized_point(
            f64::from((point.x - photo.left()).as_f32() / photo.size.width.as_f32().max(1.0)),
            f64::from((point.y - photo.top()).as_f32() / photo.size.height.as_f32().max(1.0)),
            rotation,
        )
    });
    PreviewPlacement { outside, inside }
}

/// 位图坐标 → 当前窗口坐标；每帧重新映射，窗口缩放与 rem 变化不会留下旧的缓存。
fn preview_region_bounds(
    fitted: Bounds<Pixels>,
    image_size: gpui_kit::Size<gpui_kit::DevicePixels>,
    region: TextGroupRegion,
) -> Bounds<Pixels> {
    let scale = fitted.size.width.as_f32() / image_size.width.0 as f32;
    Bounds::new(
        fitted.origin + point(px(region.x as f32 * scale), px(region.y as f32 * scale)),
        size(
            px(region.width as f32 * scale),
            px(region.height as f32 * scale),
        ),
    )
}

/// 照片按 contain 缩放到整个区域：外框固定，画面自己按比例留边。
fn render_bitmap(scene: &Arc<LayeredPreview>) -> impl IntoElement {
    let scene = scene.clone();
    let dimensions = scene.size();
    canvas(
        move |bounds, _, _| ObjectFit::Contain.get_bounds(bounds, dimensions),
        move |_, fitted, window, _| {
            let scale = fitted.size.width.as_f32() / scene.size().width.0 as f32;
            // 窗口最后叠加这些缓存纹理；未修改图层沿用同一个 RenderImage ID，
            // 无须重新上传纹理，也无须在 CPU 上压平成一张完整预览图。
            for layer in &scene.layers {
                let dimensions = layer.image.size(0);
                let rect = Bounds::new(
                    fitted.origin + point(px(layer.x as f32 * scale), px(layer.y as f32 * scale)),
                    size(
                        px(dimensions.width.0 as f32 * scale),
                        px(dimensions.height.0 as f32 * scale),
                    ),
                );
                window
                    .paint_image(
                        fitted,
                        rect,
                        Default::default(),
                        layer.image.clone(),
                        0,
                        false,
                    )
                    .ok();
            }
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        persistence::presets::WatermarkPreset,
        rotation::Rotation,
        watermark::{Placement, TextAlign, TextDirection, TextGroup, WatermarkParams},
    };
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Entity, TestAppContext, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn workspace(
        cx: &mut TestAppContext,
        groups: Vec<TextGroup>,
    ) -> (Entity<AppView>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let slot = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let slot = slot.clone();
            move |window, cx| {
                let view = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                view.update(cx, |this, cx| {
                    this.add_photos(
                        vec![
                            concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/../../test_images/DSC_4587.jpg"
                            )
                            .into(),
                            concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/../../test_images/ultra_hdr.jpg"
                            )
                            .into(),
                        ],
                        cx,
                    );
                    this.restore_watermark(
                        WatermarkPreset {
                            params: WatermarkParams::default(),
                            text_groups: groups,
                        },
                        window,
                        cx,
                    );
                });
                slot.borrow_mut().replace(view.clone());
                Root::new(view, window, cx)
            }
        });
        (slot.borrow().as_ref().unwrap().clone(), cx)
    }

    fn settle(view: &Entity<AppView>, cx: &mut VisualTestContext) {
        for _ in 0..40 {
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            if view.read_with(cx, |view, cx| {
                let preview = view.preview.read(cx);
                preview.pending.is_none() && matches!(preview.state, PreviewState::Ready(_))
            }) {
                cx.update(|window, cx| {
                    window.render_frame(cx);
                });
                return;
            }
        }
        panic!("预览未完成");
    }

    fn text_group(position: Placement) -> TextGroup {
        let mut group = TextGroup {
            position,
            ..TextGroup::default()
        };
        group.text.template = vec!["Lumen Frame".into()];
        group.text.text_params.truncate(1);
        group.text.text_params[0].size = 0.04;
        group
    }

    #[gpui_kit::test]
    fn dragging_preview_group_updates_backend_without_opening_editor(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx, vec![text_group(Placement::Bottom)]);
        settle(&view, cx);
        let id = view.read_with(cx, |view, _| view.text_groups[0].id);
        cx.update(|window, cx| {
            let image_size = view
                .read(cx)
                .preview
                .read(cx)
                .state
                .layers()
                .unwrap()
                .size();
            let fitted =
                ObjectFit::Contain.get_bounds(window.find("preview-bitmap").bounds(), image_size);
            let from = window.find(("preview-text-group", id)).bounds().center();
            let to = point(
                fitted.left() + fitted.size.width * 0.8,
                fitted.top() + fitted.size.height * 0.05,
            );
            window.drag(from, to, cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let group = &view.backend.current().text_groups[0];
            assert_eq!(
                (group.position, group.align),
                (Placement::Up, TextAlign::Right)
            );
            assert_eq!(
                group.photo_placement.as_ref().unwrap().anchor(),
                PhotoAnchor::TopRight
            );
            assert_eq!(view.text_editor_window_count(), 0);
        });
        settle(&view, cx);
        cx.update(|window, cx| {
            let layers = view
                .read(cx)
                .preview
                .read(cx)
                .state
                .layers()
                .unwrap()
                .clone();
            let fitted = ObjectFit::Contain
                .get_bounds(window.find("preview-bitmap").bounds(), layers.size());
            let photo = preview_region_bounds(fitted, layers.size(), layers.photo_region());
            let from = window.find(("preview-text-group", id)).bounds().center();
            let to = point(photo.center().x, (fitted.top() + photo.top()) / 2.);
            window.drag(from, to, cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(
                view.backend.current().text_groups[0]
                    .photo_placement
                    .is_none()
            );
            assert_eq!(view.text_editor_window_count(), 0);
        });
    }

    #[gpui_kit::test]
    fn preview_context_menu_adds_group_or_palette_at_clicked_position(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx, vec![]);
        settle(&view, cx);
        let press = cx.update(|window, cx| {
            let image_size = view
                .read(cx)
                .preview
                .read(cx)
                .state
                .layers()
                .unwrap()
                .size();
            let fitted =
                ObjectFit::Contain.get_bounds(window.find("preview-bitmap").bounds(), image_size);
            point(
                fitted.left() + fitted.size.width * 0.5,
                fitted.top() + fitted.size.height * 0.05,
            )
        });
        cx.simulate_mouse_down(press, gpui_kit::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(press, gpui_kit::MouseButton::Right, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        });
        cx.run_until_parked();
        let editor = view.read_with(cx, |view, _| {
            assert_eq!(view.backend.current().text_groups.len(), 1);
            assert_eq!(
                view.backend.current().text_groups[0].position,
                Placement::Up
            );
            *view.text_editor_windows.values().next().unwrap()
        });
        cx.update(|_, cx| {
            editor
                .update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        settle(&view, cx);
        cx.simulate_mouse_down(press, gpui_kit::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(press, gpui_kit::MouseButton::Right, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.press("down", cx);
            window.press("down", cx);
            window.press("enter", cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let card = &view.backend.params().palette_card;
            assert!(card.enabled());
            assert_eq!(card.position(), Placement::Up);
            assert!(view.palette_editor_window().is_some());
        });
    }

    #[gpui_kit::test]
    fn preview_context_menu_deletes_only_the_targeted_group(cx: &mut TestAppContext) {
        let group = text_group(Placement::Center);
        let (view, cx) = workspace(cx, vec![group.clone(), group]);
        settle(&view, cx);
        let (first, last) = view.read_with(cx, |view, _| {
            (view.text_groups[0].id, view.text_groups[1].id)
        });
        let press =
            cx.update(|window, _| window.find(("preview-text-group", last)).bounds().center());
        cx.simulate_mouse_down(press, gpui_kit::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(press, gpui_kit::MouseButton::Right, Default::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            // 最后一项是删除；菜单跳过分隔线。
            window.press("up", cx);
            window.press("enter", cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.text_groups.len(), 1);
            assert_eq!(view.text_groups[0].id, first);
            assert_eq!(view.backend.current().text_groups.len(), 1);
            assert_eq!(view.text_editor_window_count(), 0);
        });
    }

    #[gpui_kit::test]
    fn normal_window_render_does_not_flatten_layers_and_clear_releases_cache(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (view, cx) = workspace(cx, vec![text_group(Placement::Bottom)]);
        settle(&view, cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
        });
        view.read_with(cx, |view, cx| {
            let preview = view.preview.read(cx);
            assert!(preview.cache.is_some());
            assert!(!preview.state.layers().unwrap().is_flattened());
        });
        view.update_in(cx, |view, window, cx| view.clear_photos(window, cx));
        view.read_with(cx, |view, cx| {
            let preview = view.preview.read(cx);
            assert!(preview.state.layers().is_none());
            assert!(preview.cache.as_ref().unwrap().is_empty());
        });
    }

    #[gpui_kit::test]
    fn clicking_visible_groups_opens_the_correct_editor_and_reuses_it(cx: &mut TestAppContext) {
        let mut empty = text_group(Placement::Center);
        empty.text.template.clear();
        let (view, cx) = workspace(
            cx,
            vec![
                empty,
                text_group(Placement::Up),
                text_group(Placement::Bottom),
            ],
        );
        settle(&view, cx);
        let ids = view.read_with(cx, |view, _| {
            view.text_groups
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>()
        });
        cx.update(|window, cx| {
            assert!(window.try_find(("preview-text-group", ids[0])).is_none());
            window.click(("preview-text-group", ids[2]), cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.text_editor_windows.contains_key(&ids[2]));
            assert!(!view.text_editor_windows.contains_key(&ids[1]));
        });
        cx.update(|window, cx| {
            window.click(("preview-text-group", ids[2]), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.text_editor_window_count()),
            1
        );
        cx.update(|window, cx| {
            window.click(("preview-text-group", ids[1]), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.text_editor_window_count()),
            2
        );
    }

    #[gpui_kit::test]
    fn clicking_palette_opens_its_editor_and_stale_regions_are_disabled(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx, vec![text_group(Placement::Bottom)]);
        view.update_in(cx, |view, _, cx| {
            view.backend.params_mut().palette_card.set_enabled(true);
            view.backend.params_mut().palette_card.set_attachment(Some(
                crate::backend::watermark::GroupAttachment {
                    target: 0,
                    side: Placement::Right,
                    gap: 0.01,
                },
            ));
            view.backend.params_mut().rotation = Rotation::Clockwise90;
            view.refresh_preview(cx);
        });
        settle(&view, cx);
        cx.simulate_resize(size(px(1300.), px(780.)));
        cx.run_until_parked();
        let bounds = cx.update(|window, cx| {
            window.render_frame(cx);
            let preview = view.read(cx).preview.read(cx);
            let image_size = preview.state.layers().unwrap().size();
            let fitted =
                ObjectFit::Contain.get_bounds(window.find("preview-bitmap").bounds(), image_size);
            let expected =
                preview_region_bounds(fitted, image_size, preview.palette_region.unwrap());
            let actual = window.find("preview-palette-card").bounds();
            let tolerance = 0.5 / window.scale_factor() + f32::EPSILON;
            // 原点和尺寸分别取整，右/下边缘会累积两次取整误差。
            for (a, b) in [
                (actual.left(), expected.left()),
                (actual.top(), expected.top()),
                (actual.size.width, expected.size.width),
                (actual.size.height, expected.size.height),
            ] {
                assert!(
                    (a - b).as_f32().abs() <= tolerance,
                    "色卡点击区域偏移：{actual:?}，期望 {expected:?}"
                );
            }
            actual
        });
        cx.simulate_click(bounds.center(), Default::default());
        cx.run_until_parked();
        let handle = view.read_with(cx, |view, _| {
            assert_eq!(view.text_editor_window_count(), 0);
            view.palette_editor_window()
                .expect("点击色卡应打开编辑窗口")
        });
        cx.update(|window, cx| window.click("preview-palette-card", cx));
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view
                .palette_editor_window()
                .unwrap()
                .window_id()),
            handle.window_id()
        );
        cx.update(|_, cx| {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        cx.run_until_parked();
        let windows = cx.update(|_, cx| cx.windows().len());
        cx.update(|window, cx| view.update(cx, |view, cx| view.select_photo_at(1, window, cx)));
        view.read_with(cx, |view, cx| {
            assert!(view.preview.read(cx).palette_region.is_none())
        });
        cx.simulate_click(bounds.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| cx.windows().len()),
            windows,
            "旧图的色卡不能打开新照片的编辑器"
        );
        settle(&view, cx);
        cx.update(|window, _| assert!(window.try_find("preview-palette-card").is_none()));
    }

    #[gpui_kit::test]
    fn blank_preview_and_stale_photo_regions_do_not_open_editors(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx, vec![text_group(Placement::Bottom)]);
        settle(&view, cx);
        let id = view.read_with(cx, |view, _| view.text_groups[0].id);
        let (old_bounds, blank_point) = cx.update(|window, _| {
            (
                window.find(("preview-text-group", id)).bounds(),
                window.find("preview-bitmap").bounds().center(),
            )
        });
        cx.simulate_click(blank_point, Default::default());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.text_editor_window_count()),
            0
        );
        // 保留上一张位图的防抖期间，点击上一帧文字位置也不能打开新照片的编辑器。
        view.update_in(cx, |view, window, cx| view.select_photo_at(1, window, cx));
        cx.simulate_click(old_bounds.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.text_editor_window_count()),
            0
        );
        settle(&view, cx);
        let current_id = view.read_with(cx, |view, _| view.text_groups[0].id);
        cx.update(|window, cx| {
            window.click(("preview-text-group", current_id), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.text_editor_window_count()),
            1
        );
    }

    #[gpui_kit::test]
    fn overlapping_text_groups_edit_the_topmost_group(cx: &mut TestAppContext) {
        let group = text_group(Placement::Center);
        let (view, cx) = workspace(cx, vec![group.clone(), group]);
        settle(&view, cx);
        let ids = view.read_with(cx, |view, _| {
            view.text_groups
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>()
        });
        let bounds = cx.update(|window, _| window.find(("preview-text-group", ids[1])).bounds());
        cx.simulate_click(bounds.center(), Default::default());
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.text_editor_windows.contains_key(&ids[1]));
            assert!(!view.text_editor_windows.contains_key(&ids[0]));
        });
    }

    #[gpui_kit::test]
    fn rotated_vertical_text_regions_follow_window_size_and_support_keyboard(
        cx: &mut TestAppContext,
    ) {
        let group = TextGroup {
            direction: TextDirection::Vertical,
            align: TextAlign::Right,
            padding: 0.08,
            ..text_group(Placement::Right)
        };
        let (view, cx) = workspace(cx, vec![group]);
        view.update_in(cx, |view, _, cx| {
            view.backend.params_mut().rotation = Rotation::Clockwise90;
            view.refresh_preview(cx);
        });
        settle(&view, cx);
        let id = view.read_with(cx, |view, _| view.text_groups[0].id);
        cx.simulate_resize(size(px(1300.), px(780.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            let (region, image_size) = {
                let preview = view.read(cx).preview.read(cx);
                (
                    preview.text_regions[0].1,
                    preview.state.image().unwrap().size(0),
                )
            };
            let fitted =
                ObjectFit::Contain.get_bounds(window.find("preview-bitmap").bounds(), image_size);
            let actual = window.find(("preview-text-group", id)).bounds();
            let expected = preview_region_bounds(fitted, image_size, region);
            // GPUI 的布局分别对原点和尺寸取整，各自误差应小于半个设备像素。
            let tolerance = 0.5 / window.scale_factor() + f32::EPSILON;
            for (actual, expected) in [
                (actual.left(), expected.left()),
                (actual.top(), expected.top()),
                (actual.size.width, expected.size.width),
                (actual.size.height, expected.size.height),
            ] {
                assert!((actual - expected).as_f32().abs() <= tolerance);
            }
            window.blur(cx);
            for _ in 0..80 {
                window.focus_next(cx);
                window.render_frame(cx);
                if window.find(("preview-text-group", id)).focused() == Some(true) {
                    break;
                }
            }
            assert_eq!(
                window.find(("preview-text-group", id)).focused(),
                Some(true)
            );
            window.press("enter", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.text_editor_window_count()),
            1
        );
    }
}
