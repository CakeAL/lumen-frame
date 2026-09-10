//! 设置页。
//!
//! 设置整体替换中间区域，而不是叠一层抽屉：这些选项都属于「换个环境再回来干活」，值得
//! 占满工作区，也不需要再解释它们从哪儿冒出来的。
//!
//! 界面缩放直接读写 GPUI Component 的全局主题，明暗选择则通过 [`AppView::apply_appearance`]
//! 落到主题和窗口外观上，因此不存在第二份副本会与之不同步。

use gpui_kit::component::{
    ActiveTheme as _, Theme, group_box::GroupBox, h_flex, radio::RadioGroup, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, div, px};

use crate::config::AppearanceMode;

use super::AppView;

/// 界面缩放的档位。基础字号是整界面 rem 的锚点，改它会同时带动字号、间距和控件尺寸。
const INTERFACE_SCALES: &[(&str, f32)] = &[("紧凑", 14.0), ("标准", 16.0), ("宽松", 18.0)];

/// 明暗的三个选项，顺序即单选组的顺序。
const APPEARANCES: &[(&str, AppearanceMode)] = &[
    ("跟随系统", AppearanceMode::System),
    ("浅色", AppearanceMode::Light),
    ("深色", AppearanceMode::Dark),
];

impl AppView {
    pub(super) fn render_settings_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let appearance_index = APPEARANCES
            .iter()
            .position(|(_, mode)| *mode == self.appearance);
        let font_size = cx.theme().font_size.as_f32();
        let scale_index = INTERFACE_SCALES
            .iter()
            .position(|(_, size)| (size - font_size).abs() < 0.5);

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("设置"),
                    ),
            )
            .child(
                v_flex()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .mx_auto()
                            .gap_6()
                            .p_6()
                            .child(
                                GroupBox::new()
                                    .id("settings-appearance")
                                    .title("外观")
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().foreground)
                                                    .child("主题"),
                                            )
                                            .child(
                                                RadioGroup::horizontal("settings-mode")
                                                    .children(
                                                        APPEARANCES
                                                            .iter()
                                                            .map(|(label, _)| *label)
                                                            .collect::<Vec<_>>(),
                                                    )
                                                    .selected_index(appearance_index)
                                                    .on_click(cx.listener(
                                                        |this, index: &usize, window, cx| {
                                                            let Some((_, mode)) =
                                                                APPEARANCES.get(*index)
                                                            else {
                                                                return;
                                                            };
                                                            this.set_appearance_mode(
                                                                *mode, window, cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        "跟随系统时会随 macOS 的浅色/深色自动切换。",
                                                    ),
                                            )
                                            .when_some(
                                                self.settings_feedback.clone(),
                                                |this, feedback| {
                                                    this.child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(cx.theme().danger)
                                                            .child(feedback),
                                                    )
                                                },
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().foreground)
                                                    .child("界面缩放"),
                                            )
                                            .child(
                                                RadioGroup::horizontal("settings-scale")
                                                    .children(
                                                        INTERFACE_SCALES
                                                            .iter()
                                                            .map(|(label, _)| *label)
                                                            .collect::<Vec<_>>(),
                                                    )
                                                    .selected_index(scale_index)
                                                    .on_click(cx.listener(
                                                        |_, index: &usize, window, cx| {
                                                            let (_, size) =
                                                                INTERFACE_SCALES[*index];
                                                            // 基础字号是像素锚点，这一处
                                                            // `px` 是刻意的例外：它定义
                                                            // 其余相对刻度的基准。
                                                            Theme::global_mut(cx).font_size =
                                                                px(size);
                                                            Theme::sync_base(cx);
                                                            window.refresh();
                                                        },
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("缩放会同时改变字号、间距与控件尺寸。"),
                                            ),
                                    ),
                            )
                            .child(
                                GroupBox::new()
                                    .id("settings-about")
                                    .title("关于")
                                    .child(div().text_sm().text_color(cx.theme().foreground).child(
                                        format!("Lumen Frame {}", env!("CARGO_PKG_VERSION")),
                                    ))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                "用 libvips 为照片加上边框、阴影与 EXIF 文字水印。",
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}
