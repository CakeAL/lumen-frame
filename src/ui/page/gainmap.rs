//! HDR gain map 解析页面。

use std::path::{Path, PathBuf};

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, button::Button, h_flex,
    radio::RadioGroup, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, ExternalPaths, FontWeight, ObjectFit, div, img};

use super::super::AppView;
use super::super::component::field::rgb_to_hsla;
use super::super::component::gainmap_preview::GainMapPreviewState;

/// HDR 解析页自己的短生命周期状态。它不属于水印工作区，也不会进入照片队列。
pub(in crate::ui::app) struct GainMapPageState {
    path: Option<PathBuf>,
    preview: Entity<super::super::component::gainmap_preview::GainMapPreview>,
    show_gainmap: bool,
}

impl GainMapPageState {
    pub(in crate::ui::app) fn new(cx: &mut Context<AppView>) -> Self {
        Self {
            path: None,
            preview: cx.new(|_| super::super::component::gainmap_preview::GainMapPreview::new()),
            show_gainmap: true,
        }
    }

    pub(in crate::ui::app) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(in crate::ui::app) fn preview(
        &self,
    ) -> &Entity<super::super::component::gainmap_preview::GainMapPreview> {
        &self.preview
    }

    pub(in crate::ui::app) fn show_gainmap(&self) -> bool {
        self.show_gainmap
    }

    pub(in crate::ui::app) fn select(&mut self, path: PathBuf, cx: &mut Context<AppView>) {
        self.path = Some(path);
        self.request_preview(cx);
    }

    pub(in crate::ui::app) fn set_show_gainmap(
        &mut self,
        show_gainmap: bool,
        cx: &mut Context<AppView>,
    ) {
        if self.show_gainmap != show_gainmap {
            self.show_gainmap = show_gainmap;
            self.request_preview(cx);
        }
    }

    fn request_preview(&self, cx: &mut Context<AppView>) {
        let Some(path) = self.path.clone() else {
            self.preview.update(cx, |preview, cx| preview.clear(cx));
            return;
        };
        self.preview.update(cx, |preview, cx| {
            preview.request(path, self.show_gainmap, cx);
        });
    }
}

impl AppView {
    pub(in crate::ui::app) fn render_gainmap_page(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.render_gainmap_toolbar(cx))
            // 和水印页一样，预览区域本身就是拖放目标；不用用户把文件拖到一个很小的控件上。
            .child(self.render_gainmap_canvas(cx))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_gainmap_photo(paths.paths().to_vec(), cx);
            }))
    }

    fn render_gainmap_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .h_12()
            .flex_shrink_0()
            .justify_between()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child("HDR Gain Map"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("gainmap-open")
                            .label("选择照片…")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.pick_gainmap_photo(cx))),
                    )
                    .child(
                        RadioGroup::horizontal("gainmap-view")
                            .children(vec!["原图", "Gain Map"])
                            .selected_index(Some(if self.gainmap.show_gainmap() { 1 } else { 0 }))
                            .on_click(cx.listener(|this, index: &usize, _, cx| {
                                this.set_gainmap_view(*index == 1, cx);
                            })),
                    )
                    .child(
                        Button::new("gainmap-export")
                            .label("导出 Gain Map…")
                            .small()
                            .disabled(self.gainmap.path().is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_selected_gainmap(window, cx)
                            })),
                    ),
            )
    }

    fn render_gainmap_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let content = match self.gainmap.preview().read(cx).state() {
            GainMapPreviewState::Empty => div()
                .text_color(cx.theme().muted_foreground)
                .child("添加或选择一张照片以解析 HDR Gain Map")
                .into_any_element(),
            GainMapPreviewState::Loading => div()
                .text_color(cx.theme().muted_foreground)
                .child("正在解析 Gain Map…")
                .into_any_element(),
            GainMapPreviewState::Missing => div()
                .v_flex()
                .items_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::TriangleAlert))
                .child("这张图片没有 Gain Map")
                .into_any_element(),
            GainMapPreviewState::Failed(message) => div()
                .text_color(cx.theme().danger)
                .child(message.clone())
                .into_any_element(),
            GainMapPreviewState::Ready(image) => img(image.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .bg(rgb_to_hsla(self.preview_background))
            .p_6()
            .child(content)
    }
}
