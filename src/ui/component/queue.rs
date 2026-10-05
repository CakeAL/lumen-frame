//! 中间下方的照片队列。
//!
//! 队列是「待处理对象」的集合：拖入或选择进来的照片按顺序排在这里，点一张就切到它。
//! 每张卡片的缩略图单独加载，不会为了显示一排小图去解码整张原图。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    kbd::Kbd,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, FontWeight, KeyBinding, KeyDownEvent, Keystroke, ObjectFit, Role, Window, div,
    img,
};

use super::super::{AppView, Thumbnail};
use crate::workspace::QueuedPhoto;

gpui_kit::actions!(photo_queue, [PreviousPhoto, NextPhoto]);

const PREVIOUS_PHOTO_KEY: &str = "left";
const NEXT_PHOTO_KEY: &str = "right";

pub(in crate::ui::app) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(PREVIOUS_PHOTO_KEY, PreviousPhoto, Some("PhotoQueue")),
        KeyBinding::new(NEXT_PHOTO_KEY, NextPhoto, Some("PhotoQueue")),
    ]);
}

/// 队列接受的图片扩展名。
///
/// 不直接读 libvips 支持的格式列表：这里同时是给用户的提示——拖入非图片文件时会被
/// 安静地忽略，而不是排进队列后在预览里报错。
// macOS 随包使用完整的 Homebrew libvips；Windows web 包已验证 AVIF，HEIC 和相机 RAW 尚未支持。
#[cfg(target_os = "macos")]
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "tif", "tiff", "heic", "heif", "avif", "bmp", "gif", "raf",
    "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "pef", "srw",
];
#[cfg(not(target_os = "macos"))]
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "tif", "tiff", "avif", "bmp", "gif",
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
            .id("photo-queue")
            .track_focus(&self.queue_focus)
            .tab_index(0)
            .key_context("PhotoQueue")
            .role(Role::Region)
            .aria_label("照片队列，使用左右方向键切换照片")
            .on_action(cx.listener(|this, _: &PreviousPhoto, window, cx| {
                this.select_adjacent_photo(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextPhoto, window, cx| {
                this.select_adjacent_photo(false, window, cx);
            }))
            .w_full()
            .flex_shrink_0()
            .bg(cx.theme().background)
            .border_t_1()
            .border_color(cx.theme().border)
            .focus_visible(|this| this.border_color(cx.theme().ring))
            .child(self.render_queue_header(cx))
            .child(self.render_queue_strip(cx))
    }

    fn select_adjacent_photo(
        &mut self,
        previous: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .photos()
            .iter()
            .position(|photo| Some(photo.id()) == self.selected_photo_id())
        else {
            return;
        };
        let next = if previous {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        if let Some(next) = next.filter(|next| *next < self.photos().len()) {
            self.select_photo_at(next, window, cx);
            self.queue_scroll.scroll_to_item(next);
            window.focus(&self.queue_focus, cx);
        }
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
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Kbd::new(
                                Keystroke::parse(PREVIOUS_PHOTO_KEY).expect("有效的方向键"),
                            ))
                            .child(Kbd::new(
                                Keystroke::parse(NEXT_PHOTO_KEY).expect("有效的方向键"),
                            )),
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
                            .on_click(
                                cx.listener(|this, _, window, cx| this.remove_selected(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("queue-clear")
                            .label("清空")
                            .ghost()
                            .small()
                            .disabled(self.photos().is_empty())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clear_photos(window, cx)),
                            ),
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
            .track_scroll(&self.queue_scroll)
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
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_photo(id, window, cx);
                window.focus(&this.queue_focus, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.select_photo(id, window, cx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Entity, Focusable as _, TestAppContext};
    use std::{cell::RefCell, path::Path, rc::Rc};

    #[gpui_kit::test]
    fn arrow_keys_switch_photos_only_in_the_queue(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let view = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let view = view.clone();
            move |window, cx| {
                let app = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                view.borrow_mut().replace(app.clone());
                Root::new(app, window, cx)
            }
        });
        let view: Entity<AppView> = view.borrow().clone().unwrap();
        view.update_in(cx, |view, window, cx| window.focus(&view.queue_focus, cx));
        cx.update(|window, cx| window.press("right", cx));
        assert_eq!(view.read_with(cx, |view, _| view.photo_count()), 0);

        let first = Path::new("test_images/DSC_4587.jpg").to_path_buf();
        let second = Path::new("test_images/ultra_hdr.jpg").to_path_buf();
        view.update_in(cx, |view, window, cx| {
            view.add_photos(vec![first.clone(), second.clone()], cx);
            view.params.rotation = crate::rotation::Rotation::Clockwise90;
            window.focus(&view.queue_focus, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.press("left", cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_path().map(Path::to_path_buf)),
            Some(first.clone())
        );
        cx.update(|window, cx| window.press("right", cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_path().map(Path::to_path_buf)),
            Some(second.clone())
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.params.rotation.degrees()),
            0
        );
        cx.update(|window, cx| window.press("right", cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_path().map(Path::to_path_buf)),
            Some(second)
        );
        cx.update(|window, cx| window.press("left", cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_path().map(Path::to_path_buf)),
            Some(first.clone())
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.params.rotation.degrees()),
            90
        );

        view.update_in(cx, |view, window, cx| {
            view.controls
                .preset_name
                .update(cx, |input, cx| window.focus(&input.focus_handle(cx), cx));
        });
        cx.update(|window, cx| window.press("right", cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_path().map(Path::to_path_buf)),
            Some(first)
        );
    }

    #[test]
    fn photo_queue_accepts_common_formats() {
        for name in [
            "photo.jpg",
            "photo.PNG",
            "photo.webp",
            "photo.tiff",
            "photo.avif",
            "photo.bmp",
            "photo.gif",
        ] {
            assert!(is_supported_image(Path::new(name)), "{name}");
        }
        assert!(!is_supported_image(Path::new("photo.txt")));
    }

    #[test]
    fn photo_queue_matches_platform_format_support() {
        for name in [
            "photo.heic",
            "photo.heif",
            "photo.raf",
            "photo.cr2",
            "photo.cr3",
            "photo.nef",
            "photo.arw",
            "photo.dng",
            "photo.orf",
            "photo.rw2",
            "photo.pef",
            "photo.srw",
        ] {
            assert_eq!(
                is_supported_image(Path::new(name)),
                cfg!(target_os = "macos"),
                "{name}"
            );
        }
    }
}
