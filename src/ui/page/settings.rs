//! 设置页。
//!
//! 设置整体替换中间区域，而不是叠一层抽屉：这些选项都属于「换个环境再回来干活」，值得
//! 占满工作区，也不需要再解释它们从哪儿冒出来的。
//!
//! 界面缩放直接读写 GPUI Component 的全局主题，明暗选择则通过 [`AppView::apply_appearance`]
//! 落到主题和窗口外观上，因此不存在第二份副本会与之不同步。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    radio::RadioGroup,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, FontWeight, SharedString, Subscription, Window, div, px};

use crate::persistence::settings::AppearanceMode;
use crate::ui::image::PREVIEW_DEFAULT_MAX_EDGE;

use super::super::component::field::{ColorField, NumberField, field};
use super::super::{AppView, UpdateState};

/// 界面缩放的档位。基础字号是整界面 rem 的锚点，改它会同时带动字号、间距和控件尺寸。
const INTERFACE_SCALES: &[(&str, f32)] = &[("紧凑", 14.0), ("标准", 16.0), ("宽松", 18.0)];

/// 预览底图长边上限的可用范围（px）。
const PREVIEW_MAX_EDGE_MIN: i32 = 160;
const PREVIEW_MAX_EDGE_MAX: i32 = 8192;

/// 默认档位的字号。
pub(in crate::ui::app) const DEFAULT_INTERFACE_SCALE: f32 = 16.0;
pub(in crate::ui::app) const DEFAULT_PREVIEW_MAX_EDGE: i32 = PREVIEW_DEFAULT_MAX_EDGE;

pub(in crate::ui::app) fn preview_max_edge_from_settings(max_edge: Option<i32>) -> i32 {
    max_edge
        .filter(|max_edge| (PREVIEW_MAX_EDGE_MIN..=PREVIEW_MAX_EDGE_MAX).contains(max_edge))
        .unwrap_or(DEFAULT_PREVIEW_MAX_EDGE)
}

/// 把界面缩放落到全局主题上。
///
/// 基础字号是像素锚点，这一处 `px` 是刻意的例外：它定义其余相对刻度的基准。
pub(in crate::ui::app) fn apply_interface_scale(scale: f32, window: &mut Window, cx: &mut App) {
    Theme::global_mut(cx).font_size = px(scale);
    Theme::sync_base(cx);
    window.refresh();
}

/// 配色下拉的选项类型。
type ThemeSelect = SelectState<Vec<SharedString>>;

/// 设置页上的控件状态。
pub(in crate::ui::app) struct SettingsControls {
    pub light_theme: Entity<ThemeSelect>,
    pub dark_theme: Entity<ThemeSelect>,
    pub preview_background: ColorField,
    pub preview_max_edge: NumberField,
}

impl SettingsControls {
    pub(in crate::ui::app) fn new(
        preview_background_rgb: [u8; 3],
        preview_max_edge: i32,
        saved_light_theme: Option<&str>,
        saved_dark_theme: Option<&str>,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> (Self, Vec<Subscription>) {
        let light_theme = make_theme_select(ThemeMode::Light, saved_light_theme, window, cx);
        let dark_theme = make_theme_select(ThemeMode::Dark, saved_dark_theme, window, cx);
        let preview_background = ColorField::new(preview_background_rgb, window, cx);
        let preview_max_edge = NumberField::new(
            f64::from(preview_max_edge),
            f64::from(PREVIEW_MAX_EDGE_MIN),
            f64::from(PREVIEW_MAX_EDGE_MAX),
            10.0,
            0,
            1.0,
            "px",
            window,
            cx,
        );

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
        subscriptions.extend(preview_max_edge.subscribe(window, cx, |this, max_edge| {
            this.set_preview_max_edge(max_edge.round() as i32);
        }));

        (
            Self {
                light_theme,
                dark_theme,
                preview_background,
                preview_max_edge,
            },
            subscriptions,
        )
    }
}

fn make_theme_select(
    mode: ThemeMode,
    saved_name: Option<&str>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Entity<ThemeSelect> {
    let names = crate::theme::themes_for(mode, cx);
    let selected = theme_index(&names, mode, saved_name);
    cx.new(|cx| SelectState::new(names, selected, window, cx).searchable(true))
}

/// 默认选中哪一项：优先默认的那套配色，列表非空时退回第一项。
fn theme_index(
    names: &[SharedString],
    mode: ThemeMode,
    saved_name: Option<&str>,
) -> Option<IndexPath> {
    if let Some(saved_name) = saved_name
        && let Some(index) = names.iter().position(|name| name.as_ref() == saved_name)
    {
        return Some(IndexPath::new(index));
    }
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

    pub(in crate::ui::app) fn render_settings_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let appearance_index = APPEARANCES
            .iter()
            .position(|(_, mode)| *mode == self.appearance);
        let scale_index = INTERFACE_SCALES
            .iter()
            .position(|(_, size)| (size - self.interface_scale).abs() < 0.5);
        let (update_label, update_status, update_disabled): (SharedString, SharedString, bool) =
            match &self.update_state {
                UpdateState::Idle => ("检查更新".into(), "默认每 7 天自动检查一次".into(), false),
                UpdateState::Checking => {
                    ("正在检查…".into(), "正在连接 GitHub Releases".into(), true)
                }
                UpdateState::UpToDate => ("再次检查".into(), "当前已是最新版本".into(), false),
                UpdateState::Available { version } => (
                    format!("安装 {version}").into(),
                    format!("发现新版本 {version}").into(),
                    false,
                ),
                UpdateState::Installing { version } => (
                    "正在安装…".into(),
                    format!("正在下载并安装 {version}").into(),
                    true,
                ),
                UpdateState::Installed { version } => (
                    "已安装".into(),
                    format!("已安装 {version}，重启应用后生效").into(),
                    true,
                ),
                UpdateState::Failed { message } => ("重试".into(), message.clone(), false),
            };

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
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .id("settings-preferences")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .gap_6()
                            .p_6()
                            .child(
                                GroupBox::new()
                                    .id("settings-preview")
                                    .title("预览")
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .gap_2()
                                                    .child(
                                                        self.settings.preview_max_edge.render(
                                                            "预览底图长边上限",
                                                            false,
                                                            cx,
                                                        ),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(
                                                                "该值会影响预览渲染速度；数值越小越快，且不影响导出图片。",
                                                            ),
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
                                            ),
                            )
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
                                            .child(
                                                h_flex()
                                                    .child(self.render_theme_slot(ThemeMode::Light, cx))
                                                    .gap_2()
                                                    .child(self.render_theme_slot(ThemeMode::Dark, cx))
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        "浅色与深色各选一套配色；切换明暗时用对应的那一套。",
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
                                                            "恢复为跟随系统、默认配色、标准缩放与默认预览分辨率；不影响水印参数和预设。",
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
                            ),
                    )
                    .child(
                        v_flex()
                            .w_1_3()
                            .flex_shrink_0()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .p_6()
                            .child(
                                v_flex()
                                    .w_full()
                                    .child(
                                        GroupBox::new()
                                            .id("settings-about")
                                            .title("关于")
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().foreground)
                                                    .child(format!(
                                                        "Lumen Frame {}",
                                                        env!("CARGO_PKG_VERSION")
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        "基于 libvips 为照片加上边框、阴影与 EXIF 文字水印。",
                                                    ),
                                            )
                                            .child(
                                                v_flex()
                                                    .w_full()
                                                    .gap_2()
                                                    .pt_2()
                                                    .child(
                                                        Button::new("settings-check-update")
                                                            .label(update_label)
                                                            .w_full()
                                                            .disabled(update_disabled)
                                                            .on_click(cx.listener(
                                                                |this, _, window, cx| {
                                                                    this.check_for_updates(
                                                                        window, cx,
                                                                    )
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(update_status),
                                                    ),
                                            ),
                            ),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_theme_is_selected_before_the_default() {
        let names = vec![
            SharedString::from("Default Light"),
            SharedString::from("Catppuccin Latte"),
        ];

        assert_eq!(
            theme_index(&names, ThemeMode::Light, Some("Catppuccin Latte")),
            Some(IndexPath::new(1))
        );
    }
}
