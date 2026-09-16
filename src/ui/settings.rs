//! 设置页。
//!
//! 设置整体替换中间区域，而不是叠一层抽屉：这些选项都属于「换个环境再回来干活」，值得
//! 占满工作区，也不需要再解释它们从哪儿冒出来的。
//!
//! 界面缩放直接读写 GPUI Component 的全局主题，明暗选择则通过 [`AppView::apply_appearance`]
//! 落到主题和窗口外观上，因此不存在第二份副本会与之不同步。

use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    radio::RadioGroup,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, FontWeight, SharedString, Subscription, Window, div, px};

use crate::config::AppearanceMode;

use super::AppView;
use super::field::{ColorField, field};

/// 界面缩放的档位。基础字号是整界面 rem 的锚点，改它会同时带动字号、间距和控件尺寸。
const INTERFACE_SCALES: &[(&str, f32)] = &[("紧凑", 14.0), ("标准", 16.0), ("宽松", 18.0)];

/// 默认档位的字号。
pub(super) const DEFAULT_INTERFACE_SCALE: f32 = 16.0;

/// 把界面缩放落到全局主题上。
///
/// 基础字号是像素锚点，这一处 `px` 是刻意的例外：它定义其余相对刻度的基准。
pub(super) fn apply_interface_scale(scale: f32, window: &mut Window, cx: &mut App) {
    Theme::global_mut(cx).font_size = px(scale);
    Theme::sync_base(cx);
    window.refresh();
}

/// 配色下拉的选项类型。
type ThemeSelect = SelectState<Vec<SharedString>>;

/// 设置页上的控件状态。
pub(super) struct SettingsControls {
    pub light_theme: Entity<ThemeSelect>,
    pub dark_theme: Entity<ThemeSelect>,
    pub preview_background: ColorField,
}

impl SettingsControls {
    pub(super) fn new(
        preview_background_rgb: [u8; 3],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> (Self, Vec<Subscription>) {
        let light_theme = make_theme_select(ThemeMode::Light, window, cx);
        let dark_theme = make_theme_select(ThemeMode::Dark, window, cx);
        let preview_background = ColorField::new(preview_background_rgb, window, cx);

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe_in(&light_theme, window, |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(name)) = event else {
                    return;
                };
                this.set_theme_slot(ThemeMode::Light, name.clone(), window, cx);
            }),
        );
        subscriptions.push(
            cx.subscribe_in(&dark_theme, window, |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(name)) = event else {
                    return;
                };
                this.set_theme_slot(ThemeMode::Dark, name.clone(), window, cx);
            }),
        );
        subscriptions.extend(preview_background.subscribe(window, cx, |this, rgb| {
            this.set_preview_background(rgb);
        }));

        (
            Self {
                light_theme,
                dark_theme,
                preview_background,
            },
            subscriptions,
        )
    }
}

fn make_theme_select(
    mode: ThemeMode,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Entity<ThemeSelect> {
    let names = crate::theme::themes_for(mode, cx);
    let selected = default_theme_index(&names, mode);
    cx.new(|cx| SelectState::new(names, selected, window, cx).searchable(true))
}

/// 默认选中哪一项：优先默认的那套配色，列表非空时退回第一项。
fn default_theme_index(names: &[SharedString], mode: ThemeMode) -> Option<IndexPath> {
    let fallback = if mode == ThemeMode::Dark {
        "Default Dark"
    } else {
        "Default Light"
    };
    names
        .iter()
        .position(|name| name.as_ref() == fallback)
        .or(if names.is_empty() { None } else { Some(0) })
        .map(IndexPath::new)
}

/// 明暗的三个选项，顺序即单选组的顺序。
const APPEARANCES: &[(&str, AppearanceMode)] = &[
    ("跟随系统", AppearanceMode::System),
    ("浅色", AppearanceMode::Light),
    ("深色", AppearanceMode::Dark),
];

impl AppView {
    /// 一个配色槽位的下拉。两个槽位长一样，只有标题和来源状态不同。
    fn render_theme_slot(&self, mode: ThemeMode, cx: &Context<Self>) -> impl IntoElement {
        let (label, state) = match mode {
            ThemeMode::Light => ("浅色主题", &self.settings.light_theme),
            ThemeMode::Dark => ("深色主题", &self.settings.dark_theme),
        };
        field(label, Select::new(state).w_full(), cx)
    }

    pub(super) fn render_settings_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let appearance_index = APPEARANCES
            .iter()
            .position(|(_, mode)| *mode == self.appearance);
        let scale_index = INTERFACE_SCALES
            .iter()
            .position(|(_, size)| (size - self.interface_scale).abs() < 0.5);

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
                    .h_12()
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
                                                        "跟随系统时会随操作系统的浅色/深色自动切换。",
                                                    ),
                                            )
                                            .child(self.render_theme_slot(ThemeMode::Light, cx))
                                            .child(self.render_theme_slot(ThemeMode::Dark, cx))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        "浅色与深色各选一套配色；切换明暗时用对应的那一套。",
                                                    ),
                                            )
                                            .child(self.settings.preview_background.render(
                                                "照片展示背景",
                                                false,
                                                cx,
                                            ))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        "只影响照片展示区域，不影响预览生成结果或导出。",
                                                    ),
                                            )
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .gap_2()
                                                    .pt_2()
                                                    .child(
                                                        Button::new("settings-reset")
                                                            .label("恢复默认设置")
                                                            .ghost()
                                                            .w_full()
                                                            .on_click(cx.listener(
                                                                |this, _, window, cx| {
                                                                    this.reset_appearance_defaults(
                                                                        window, cx,
                                                                    )
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        div().text_xs().text_color(
                                                            cx.theme().muted_foreground,
                                                        )
                                                        .child(
                                                            "恢复为跟随系统、默认配色与标准缩放；不影响水印参数和预设。",
                                                        ),
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
                                                        |this, index: &usize, window, cx| {
                                                            let (_, size) =
                                                                INTERFACE_SCALES[*index];
                                                            this.set_interface_scale(
                                                                size, window, cx,
                                                            );
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
