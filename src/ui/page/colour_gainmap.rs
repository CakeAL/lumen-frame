//! 彩色恢复 Gain Map 生成页面。

use std::path::{Path, PathBuf};

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, button::Button, h_flex,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, ExternalPaths, FontWeight, ObjectFit, div, img};

use super::super::AppView;
use super::super::component::colour_gainmap_preview::{
    ColourGainMapPreview, ColourGainMapPreviewState,
};
use super::super::component::field::rgb_to_hsla;

pub(in crate::ui::app) struct ColourGainMapPageState {
    path: Option<PathBuf>,
    preview: Entity<ColourGainMapPreview>,
    exporting: bool,
}

impl ColourGainMapPageState {
    pub(in crate::ui::app) fn new(cx: &mut Context<AppView>) -> Self {
        Self {
            path: None,
            preview: cx.new(|_| ColourGainMapPreview::new()),
            exporting: false,
        }
    }

    pub(in crate::ui::app) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub(in crate::ui::app) fn preview(&self) -> &Entity<ColourGainMapPreview> {
        &self.preview
    }
    pub(in crate::ui::app) fn is_exporting(&self) -> bool {
        self.exporting
    }

    pub(in crate::ui::app) fn select(&mut self, path: PathBuf, cx: &mut Context<AppView>) {
        self.path = Some(path.clone());
        self.preview
            .update(cx, |preview, cx| preview.request(path, cx));
    }

    pub(in crate::ui::app) fn set_exporting(&mut self, exporting: bool, cx: &mut Context<AppView>) {
        if self.exporting != exporting {
            self.exporting = exporting;
            cx.notify();
        }
    }
}

impl AppView {
    pub(in crate::ui::app) fn render_colour_gainmap_page(
        &self,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.render_colour_gainmap_toolbar(cx))
            .child(self.render_colour_gainmap_canvas(cx))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_colour_gainmap_photo(paths.paths().to_vec(), cx);
            }))
    }

    fn render_colour_gainmap_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
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
                    .child("彩色恢复 Gain Map"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("colour-gainmap-open")
                            .label("选择照片…")
                            .small()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.pick_colour_gainmap_photo(cx)),
                            ),
                    )
                    .child(
                        Button::new("colour-gainmap-export")
                            .label(if self.colour_gainmap.is_exporting() {
                                "正在生成…"
                            } else {
                                "导出 JPEG…"
                            })
                            .small()
                            .disabled(
                                self.colour_gainmap.path().is_none()
                                    || self.colour_gainmap.is_exporting(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_colour_gainmap(window, cx)
                            })),
                    ),
            )
    }

    fn render_colour_gainmap_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let content = match self.colour_gainmap.preview().read(cx).state() {
            ColourGainMapPreviewState::Empty => div()
                .v_flex()
                .items_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Palette))
                .child("选择或拖入一张照片，生成黑白底图和彩色恢复 Gain Map")
                .into_any_element(),
            ColourGainMapPreviewState::Loading => div()
                .text_color(cx.theme().muted_foreground)
                .child("正在生成预览…")
                .into_any_element(),
            ColourGainMapPreviewState::Failed(message) => div()
                .text_color(cx.theme().danger)
                .child(message.clone())
                .into_any_element(),
            ColourGainMapPreviewState::Ready(image) => img(image.clone())
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
