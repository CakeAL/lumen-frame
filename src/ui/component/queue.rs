//! 中间下方的照片队列。
//!
//! 队列是「待处理对象」的集合：拖入或选择进来的照片按顺序排在这里，点一张就切到它。
//! 每张卡片的缩略图单独加载，不会为了显示一排小图去解码整张原图。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, KeyDownEvent, ObjectFit, Role, div, img};

use super::super::{AppView, Thumbnail};
use crate::workspace::QueuedPhoto;

/// 队列接受的图片扩展名。
///
/// 不直接读 libvips 支持的格式列表：这里同时是给用户的提示——拖入非图片文件时会被
/// 安静地忽略，而不是排进队列后在预览里报错。
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "tif", "tiff", "heic", "heif", "avif", "bmp", "gif", "raf",
    "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "pef", "srw",
];

pub(in crate::ui::app) fn is_supported_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| SUPPORTED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

impl AppView {
    pub(in crate::ui::app) fn render_queue_pane(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .flex_shrink_0()
            .bg(cx.theme().background)
            .border_t_1()
            .border_color(cx.theme().border)
            .child(self.render_queue_header(cx))
            .child(self.render_queue_strip(cx))
    }

    fn render_queue_header(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .justify_between()
            .gap_3()
            .px_4()
            .py_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("照片队列"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} 张", self.photos().len())),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("queue-add")
                            .icon(IconName::Plus)
                            .label("添加照片")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.pick_photos(cx))),
                    )
                    .child(
                        Button::new("queue-remove")
                            .icon(IconName::Close)
                            .label("移除")
                            .ghost()
                            .small()
                            .disabled(self.selected_photo_id().is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.remove_selected(cx))),
                    )
                    .child(
                        Button::new("queue-clear")
                            .label("清空")
                            .ghost()
                            .small()
                            .disabled(self.photos().is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.clear_photos(cx))),
                    ),
            )
    }

    fn render_queue_strip(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .id("queue-strip")
            .w_full()
            .gap_3()
            .px_4()
            .pb_3()
            .min_h_0()
            .overflow_x_scroll()
            .when(self.photos().is_empty(), |this| {
                this.child(
                    div()
                        .w_full()
                        .py_6()
                        .text_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("拖入照片，或用「添加照片」选择；一次可以拖入多张"),
                )
            })
            .when(!self.photos().is_empty(), |this| {
                this.children(
                    self.photos()
                        .iter()
                        .map(|photo| self.render_photo_card(photo, cx)),
                )
            })
    }

    fn render_photo_card(&self, photo: &QueuedPhoto, cx: &Context<Self>) -> impl IntoElement {
        let id = photo.id();
        let is_selected = self.selected_photo_id() == Some(id);
        let name = photo
            .path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| photo.path().to_string_lossy().into_owned());

        let preview = match self.thumbnail(id).unwrap_or(&Thumbnail::Pending) {
            Thumbnail::Ready(image) => img(image.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            Thumbnail::Pending => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(Spinner::new().small())
                .into_any_element(),
            Thumbnail::Failed => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::TriangleAlert).small())
                .into_any_element(),
        };

        v_flex()
            .id(id.value() as usize)
            .flex_shrink_0()
            .w_32()
            .gap_2()
            .p_1()
            .rounded(cx.theme().radius)
            .border_2()
            .border_color(if is_selected {
                cx.theme().primary
            } else {
                cx.theme().transparent
            })
            .cursor_pointer()
            .focusable()
            .tab_index(0)
            .role(Role::Button)
            .aria_selected(is_selected)
            .aria_label(name.clone())
            .hover(|this| this.bg(cx.theme().muted))
            .focus_visible(|this| this.border_color(cx.theme().ring))
            .on_click(cx.listener(move |this, _, _, cx| this.select_photo(id, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.select_photo(id, cx);
                }
            }))
            .child(
                div()
                    .w_full()
                    .h_20()
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .bg(cx.theme().muted)
                    .child(preview),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_xs()
                    .text_color(if is_selected {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(name),
            )
    }
}
