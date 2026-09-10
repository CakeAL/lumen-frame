//! 中间上方的预览面板。
//!
//! 面板本身只负责呈现，渲染节奏由 [`WatermarkPreview`] 这个实体决定：参数每变一次就
//! 请求一次，请求在实体里合并、防抖，再到后台线程重算。这样拖动滑块时不会每帧都启动
//! 一次 libvips 合成，也不会让已经过期的结果覆盖新结果。

use std::{sync::Arc, time::Duration};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, Size, StyledExt as _, h_flex, spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, ObjectFit, RenderImage, SharedString, Task, div, img};

use super::AppView;
use super::preview_image::{PreviewJob, render_preview};

/// 参数连续变化时先攒一会儿再算。
///
/// 这个值决定「跟手」和「算得少」的平衡：太小则拖动滑块时排满后台任务，太大则有明显
/// 的延迟感。
const DEBOUNCE: Duration = Duration::from_millis(80);

/// 预览位图的状态。
///
/// 重算时保留上一张图（[`PreviewState::Rendering`] 的 `previous`），是为了让拖动滑块
/// 的过程中画面连续变化，而不是每次重算都闪一下空白。
pub enum PreviewState {
    /// 队列里没有可预览的照片。
    Empty,
    Rendering {
        previous: Option<Arc<RenderImage>>,
    },
    Ready(Arc<RenderImage>),
    Failed(SharedString),
}

impl PreviewState {
    /// 当前用于显示的位图。重算期间仍然是上一张，所以拖动滑块时画面不会闪空。
    pub(super) fn image(&self) -> Option<&Arc<RenderImage>> {
        match self {
            PreviewState::Rendering { previous } => previous.as_ref(),
            PreviewState::Ready(image) => Some(image),
            PreviewState::Empty | PreviewState::Failed(_) => None,
        }
    }
}

/// 预览的渲染节奏与结果。
pub struct WatermarkPreview {
    state: PreviewState,
    /// 每次请求自增。渲染完成时对不上，就说明这份结果已经过期，直接丢弃。
    generation: u64,
    /// 还没交给后台线程的最新一次请求。
    pending: Option<PreviewJob>,
    worker: Option<Task<()>>,
}

impl WatermarkPreview {
    pub fn new() -> Self {
        Self {
            state: PreviewState::Empty,
            generation: 0,
            pending: None,
            worker: None,
        }
    }

    pub fn state(&self) -> &PreviewState {
        &self.state
    }

    /// 请求一次预览。同一轮防抖内的多次请求会合并成最后一次。
    pub fn request(&mut self, job: PreviewJob, cx: &mut Context<Self>) {
        self.pending = Some(job);
        self.generation = self.generation.wrapping_add(1);

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

                let Some(job) = this
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
                        previous: this.state.image().cloned(),
                    };
                    cx.notify();
                });
                if started.is_err() {
                    break;
                }

                let rendered = cx
                    .background_spawn(async move { render_preview(&job) })
                    .await;

                let applied = this.update(cx, |this, cx| {
                    // 期间用户又改了参数：这份结果对应的已经不是界面上的状态了。
                    if this.generation != generation {
                        return;
                    }
                    this.state = match rendered {
                        Ok(image) => PreviewState::Ready(image),
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
        self.generation = self.generation.wrapping_add(1);
        if !matches!(self.state, PreviewState::Empty) {
            self.state = PreviewState::Empty;
        }
        cx.notify();
    }
}

impl AppView {
    pub(super) fn render_preview_pane(&self, cx: &Context<Self>) -> impl IntoElement {
        let title = self.selected_photo().map(|photo| {
            photo
                .path
                .file_name()
                .map(|name| SharedString::from(name.to_string_lossy().into_owned()))
                .unwrap_or_else(|| SharedString::from(photo.path.to_string_lossy().into_owned()))
        });

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().background)
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
            .py_2()
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
            .child(self.render_preview_status(cx))
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
                .child("正在更新预览")
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
                .child(
                    div()
                        .text_sm()
                        .child("把照片拖到这里，或用下方的「添加照片」选择"),
                )
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
            PreviewState::Ready(image) => render_bitmap(image).into_any_element(),
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
            .p_6()
            .child(content)
    }
}

/// 照片按 contain 缩放到整个区域：外框固定，画面自己按比例留边。
fn render_bitmap(image: &Arc<RenderImage>) -> impl IntoElement {
    img(image.clone())
        .size_full()
        .object_fit(ObjectFit::Contain)
}
