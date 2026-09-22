//! 预设面板与右侧参数面板。
//!
//! 面板里的控件实体只保存控件自身的状态（滑块位置、下拉框开合、取色器面板），参数值
//! 始终以 [`AppView::params`] 为准：控件回调写入参数，然后请求一次预览重算。这样预览、
//! 导出、界面读数永远来自同一份数据。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    accordion::Accordion,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, Hsla, IntoElement, KeyDownEvent, Role, SharedString,
    Subscription, Window, black, div, linear_color_stop, linear_gradient, relative, rgba, white,
};

use crate::watermark::{Placement, TextAlign, TextDirection, TextGroup, WatermarkParams};

use super::super::{AppView, ExportState};
use super::field::{
    Choice, ColorField, NumberField, choices, field, hint, index_of, on_select, rgb_to_hsla,
    select_state, warning,
};

fn preset_text_marker(group: &TextGroup, color: Hsla) -> AnyElement {
    let vertical = group.direction == TextDirection::Vertical;
    let align_offset = match group.align {
        TextAlign::Left => 0.08,
        TextAlign::Center => 0.36,
        TextAlign::Right => 0.64,
    };

    if vertical {
        let left = match group.position {
            Placement::Left => 0.04,
            Placement::Right => 0.935,
            Placement::Up | Placement::Bottom | Placement::Center => align_offset + 0.12,
        };
        let top = match group.position {
            Placement::Up => 0.08,
            Placement::Bottom => 0.62,
            Placement::Center => 0.35,
            Placement::Left | Placement::Right => match group.align {
                TextAlign::Left => 0.12,
                TextAlign::Center => 0.35,
                TextAlign::Right => 0.58,
            },
        };
        div()
            .absolute()
            .w(relative(0.025))
            .h(relative(0.30))
            .left(relative(left))
            .top(relative(top))
            .rounded_full()
            .bg(color)
            .into_any_element()
    } else {
        let left = match group.position {
            Placement::Left => 0.05,
            Placement::Right => 0.67,
            Placement::Up | Placement::Bottom | Placement::Center => align_offset,
        };
        let top = match group.position {
            Placement::Up => 0.05,
            Placement::Bottom => 0.90,
            Placement::Center | Placement::Left | Placement::Right => 0.48,
        };
        div()
            .absolute()
            .w(relative(0.28))
            .h(relative(0.035))
            .left(relative(left))
            .top(relative(top))
            .rounded_full()
            .bg(color)
            .into_any_element()
    }
}

fn preset_section_label(label: &'static str, cx: &Context<AppView>) -> impl IntoElement {
    div()
        .mt_2()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

/// 模糊强度的上限。
///
/// 再往上 blur 的耗时涨得比效果快，实际也很难看出差别，所以界面上就收在 150。
const BLUR_MAX: f64 = 150.0;

/// 宽高比的选项：不限制、常用比例、自定义。
///
/// 「不动宽高比」和「指定一个比例」是两件事，所以不强制比例的选项叫「不限制」。
#[derive(Clone, PartialEq)]
pub(in crate::ui::app) enum AspectRatioChoice {
    /// 不限制：画布尺寸只跟照片和边框有关。
    Free,
    Preset(f64, f64),
    /// 自定义比例，具体数值在旁边两个输入框里。
    Custom,
}

pub(in crate::ui::app) const ASPECT_RATIOS: &[(&str, AspectRatioChoice)] = &[
    ("不限制", AspectRatioChoice::Free),
    ("1:1", AspectRatioChoice::Preset(1.0, 1.0)),
    ("4:5", AspectRatioChoice::Preset(4.0, 5.0)),
    ("5:4", AspectRatioChoice::Preset(5.0, 4.0)),
    ("3:2", AspectRatioChoice::Preset(3.0, 2.0)),
    ("2:3", AspectRatioChoice::Preset(2.0, 3.0)),
    ("16:9", AspectRatioChoice::Preset(16.0, 9.0)),
    ("9:16", AspectRatioChoice::Preset(9.0, 16.0)),
    ("自定义", AspectRatioChoice::Custom),
];

/// 图片与文字水印可以贴的位置。
pub(super) const POSITIONS: &[(&str, Placement)] = &[
    ("居中", Placement::Center),
    ("靠上", Placement::Up),
    ("靠下", Placement::Bottom),
    ("靠左", Placement::Left),
    ("靠右", Placement::Right),
];

/// 下拉框状态的具体类型别名，免得这串泛型在签名里反复出现。
pub(in crate::ui::app) type AspectRatioSelect = SelectState<Vec<Choice<AspectRatioChoice>>>;
pub(in crate::ui::app) type PositionSelect = SelectState<Vec<Choice<Placement>>>;

/// 面板里所有需要跨帧保留的控件状态。
pub(in crate::ui::app) struct ParameterControls {
    pub border_top: NumberField,
    pub border_bottom: NumberField,
    pub border_left: NumberField,
    pub border_right: NumberField,
    pub border_radius: NumberField,
    pub shadow_size: NumberField,
    pub shadow_density: NumberField,
    pub blur_sigma: NumberField,
    pub quality: NumberField,
    pub background: ColorField,

    pub aspect_ratio: Entity<AspectRatioSelect>,
    /// 自定义宽高比的两个输入框。
    pub aspect_width: Entity<InputState>,
    pub aspect_height: Entity<InputState>,
    pub position: Entity<PositionSelect>,
    pub output_folder: Entity<InputState>,
    pub preset_name: Entity<InputState>,
}

impl ParameterControls {
    /// 创建全部控件，并把「控件变化 → 写回参数 → 请求预览」这条链路一次接好。
    pub(in crate::ui::app) fn new(
        params: &WatermarkParams,
        aspect_choice: &AspectRatioChoice,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> (Self, Vec<Subscription>) {
        let mut subscriptions = Vec::new();

        let border_top = NumberField::new(
            params.border_ratio.0,
            0.0,
            40.0,
            0.5,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let border_bottom = NumberField::new(
            params.border_ratio.1,
            0.0,
            40.0,
            0.5,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let border_left = NumberField::new(
            params.border_ratio.2,
            0.0,
            40.0,
            0.5,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let border_right = NumberField::new(
            params.border_ratio.3,
            0.0,
            40.0,
            0.5,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let border_radius = NumberField::new(
            params.border_radius,
            0.0,
            20.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let shadow_size = NumberField::new(
            params.shadow_size,
            0.0,
            30.0,
            0.5,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let shadow_density = NumberField::new(
            params.shadow_density,
            0.0,
            2.0,
            0.05,
            2,
            1.0,
            "",
            window,
            cx,
        );
        let blur_sigma = NumberField::new(
            params.blur_sigma,
            0.0,
            BLUR_MAX,
            1.0,
            0,
            1.0,
            "",
            window,
            cx,
        );
        let quality = NumberField::new(
            f64::from(params.quality),
            1.0,
            100.0,
            1.0,
            0,
            1.0,
            "",
            window,
            cx,
        );
        let background = ColorField::new(params.background, window, cx);

        subscriptions.extend(border_top.subscribe(window, cx, |this, value| {
            this.params.border_ratio.0 = value;
        }));
        subscriptions.extend(border_bottom.subscribe(window, cx, |this, value| {
            this.params.border_ratio.1 = value;
        }));
        subscriptions.extend(border_left.subscribe(window, cx, |this, value| {
            this.params.border_ratio.2 = value;
        }));
        subscriptions.extend(border_right.subscribe(window, cx, |this, value| {
            this.params.border_ratio.3 = value;
        }));
        subscriptions.extend(border_radius.subscribe(window, cx, |this, value| {
            this.params.border_radius = value;
        }));
        subscriptions.extend(shadow_size.subscribe(window, cx, |this, value| {
            this.params.shadow_size = value;
        }));
        subscriptions.extend(shadow_density.subscribe(window, cx, |this, value| {
            this.params.shadow_density = value;
        }));
        subscriptions.extend(blur_sigma.subscribe(window, cx, |this, value| {
            this.params.blur_sigma = value;
        }));
        subscriptions.extend(quality.subscribe(window, cx, |this, value| {
            this.params.quality = value.round() as i32;
        }));
        subscriptions.extend(background.subscribe(window, cx, |this, rgb| {
            this.params.background = rgb;
        }));

        let aspect_ratio = select_state(
            choices(ASPECT_RATIOS),
            index_of(ASPECT_RATIOS, aspect_choice),
            window,
            cx,
        );
        let (ratio_width, ratio_height) = initial_ratio(params.aspect_ratio);
        let aspect_width = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(ratio_width)
                .placeholder("宽")
        });
        let aspect_height = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(ratio_height)
                .placeholder("高")
        });
        let position = select_state(
            choices(POSITIONS),
            index_of(POSITIONS, &params.position),
            window,
            cx,
        );

        subscriptions.push(on_select(&aspect_ratio, window, cx, |this, choice, cx| {
            this.apply_aspect_choice(choice, cx);
        }));
        subscriptions.push(on_select(&position, window, cx, |this, position, _| {
            this.params.position = position;
        }));

        // 自定义比例：两个框任意一个变了，就拿两个框当前的值重算比例。
        for input in [&aspect_width, &aspect_height] {
            subscriptions.push(cx.subscribe_in(input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_custom_aspect_ratio(cx);
                }
            }));
        }

        let output_folder = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(
                    params
                        .output_folder
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
                .placeholder("导出到哪个文件夹")
        });
        subscriptions.push(
            cx.subscribe_in(&output_folder, window, |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_string();
                    this.set_output_folder(value);
                    cx.notify();
                }
            }),
        );

        let preset_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("给这套配置起个名字"));

        (
            Self {
                border_top,
                border_bottom,
                border_left,
                border_right,
                border_radius,
                shadow_size,
                shadow_density,
                blur_sigma,
                quality,
                background,
                aspect_ratio,
                aspect_width,
                aspect_height,
                position,
                output_folder,
                preset_name,
            },
            subscriptions,
        )
    }
}

/// 没有指定比例时，自定义输入框里先放一个 1:1，省得用户面对两个空格。
fn initial_ratio(aspect_ratio: Option<(f64, f64)>) -> (String, String) {
    let (width, height) = aspect_ratio.unwrap_or((1.0, 1.0));
    (format_number(width), format_number(height))
}

pub(super) fn format_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

/// 从当前配置生成一个预设。
pub(in crate::ui::app) fn preset_of(
    params: &WatermarkParams,
    text_groups: &[TextGroup],
) -> crate::persistence::presets::WatermarkPreset {
    crate::persistence::presets::WatermarkPreset {
        params: params.clone(),
        text_groups: text_groups.to_vec(),
    }
}

impl AppView {
    pub(in crate::ui::app) fn render_inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .h_full()
            .w_80()
            .flex_shrink_0()
            .bg(cx.theme().background)
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .gap_2()
                    .px_4()
                    .h_12()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("水印参数"),
                    ),
            )
            .child(
                v_flex()
                    .id("inspector-scroll")
                    .flex_1()
                    .min_h_0()
                    .gap_6()
                    .p_4()
                    .overflow_y_scroll()
                    .child(self.render_canvas_section(cx))
                    .child(self.render_background_section(cx))
                    .child(self.render_image_section(cx))
                    .child(self.render_text_section(cx)),
            )
    }

    // MARK: 预设

    /// 左侧预设面板：两列卡片在独立滚动区内，保存与打开文件夹固定在顶部。
    pub(in crate::ui::app) fn render_preset_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.preset_panel_collapsed {
            return v_flex()
                .id("preset-panel")
                .w_12()
                .h_full()
                .flex_shrink_0()
                .items_center()
                .pt_2()
                .border_r_1()
                .border_color(cx.theme().border)
                .child(
                    Button::new("preset-panel-expand")
                        .icon(IconName::PanelLeftOpen)
                        .ghost()
                        .small()
                        .tooltip("展开预设")
                        .accessibility_label("展开预设")
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_preset_panel(cx))),
                )
                .into_any_element();
        }
        let name_ready = !self.controls.preset_name.read(cx).value().trim().is_empty();

        v_flex()
            .id("preset-panel")
            .w_72()
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().background)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .justify_between()
                    .gap_2()
                    .px_4()
                    .h_12()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().foreground)
                            .child("预设"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("preset-open-folder")
                                    .icon(IconName::FolderOpen)
                                    .ghost()
                                    .small()
                                    .tooltip("打开预设文件夹")
                                    .accessibility_label("打开预设文件夹")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_preset_folder(cx)),
                                    ),
                            )
                            .child(
                                Button::new("preset-panel-collapse")
                                    .icon(IconName::PanelLeftClose)
                                    .ghost()
                                    .small()
                                    .tooltip("收起预设")
                                    .accessibility_label("收起预设")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_preset_panel(cx)),
                                    ),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_shrink_0()
                    .gap_2()
                    .p_4()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.controls.preset_name)),
                            )
                            .child(
                                Button::new("preset-save")
                                    .label("添加")
                                    .disabled(!name_ready)
                                    .on_click(cx.listener(|this, _, _, cx| this.save_preset(cx))),
                            ),
                    )
                    .when_some(self.preset_feedback.clone(), |this, feedback| {
                        this.child(if self.preset_feedback_is_error {
                            warning(feedback, cx)
                        } else {
                            hint(feedback, cx)
                        })
                    }),
            )
            .child(
                v_flex()
                    .id("preset-cards")
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .p_4()
                    .overflow_y_scroll()
                    .child(preset_section_label("内置预设", cx))
                    .children(
                        self.preset_names
                            .iter()
                            .filter(|name| self.builtin_preset_names.contains(*name))
                            .map(|name| self.render_preset_card(name.clone(), true, cx)),
                    )
                    .when(
                        self.preset_names
                            .iter()
                            .any(|name| !self.builtin_preset_names.contains(name)),
                        |this| this.child(preset_section_label("我的预设", cx)),
                    )
                    .children(
                        self.preset_names
                            .iter()
                            .filter(|name| !self.builtin_preset_names.contains(*name))
                            .map(|name| self.render_preset_card(name.clone(), false, cx)),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .w_full()
                            .gap_3()
                            .child(
                                Button::new("preset-reset")
                                    .icon(IconName::Undo2)
                                    .label("恢复默认参数")
                                    .ghost()
                                    .w_full()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_reset_params(window, cx)
                                    })),
                            )
                            .child(self.render_output_controls(cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_preset_card(
        &self,
        name: SharedString,
        builtin: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let for_load = name.clone();
        let for_keyboard = name.clone();
        let for_delete = name.clone();
        let preview = self
            .preset_previews
            .get(&name)
            .map(|preset| self.render_preset_preview(preset, cx))
            .unwrap_or_else(|| hint("预览不可用", cx));

        v_flex()
            .relative()
            .w_full()
            .min_w_0()
            .child(
                h_flex()
                    .items_stretch()
                    .id(format!("preset-card-{name}"))
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .p_3()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().group_box)
                    .focusable()
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("载入预设 {name}"))
                    .hover(|this| {
                        this.bg(cx.theme().muted)
                            .border_color(cx.theme().primary)
                            .shadow_sm()
                    })
                    .focus_visible(|this| this.border_color(cx.theme().ring))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.load_preset(&for_load, window, cx)
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.load_preset(&for_keyboard, window, cx);
                        }
                    }))
                    .child(
                        v_flex()
                            .w_24()
                            .flex_shrink_0()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if builtin { "内置" } else { "我的预设" }),
                            )
                            .when(!builtin, |this| this.pr_8()),
                    )
                    .child(div().flex_1().min_w_0().child(preview)),
            )
            .when(!builtin, |this| {
                this.child(
                    div().absolute().right_2().bottom_2().child(
                        Button::new(format!("preset-delete-{for_delete}"))
                            .icon(IconName::Delete)
                            .ghost()
                            .small()
                            .tooltip("删除这个预设")
                            .accessibility_label(format!("删除预设 {for_delete}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.confirm_delete_preset(&for_delete, window, cx)
                            })),
                    ),
                )
            })
    }

    fn render_preset_preview(
        &self,
        preset: &crate::persistence::presets::WatermarkPreset,
        cx: &Context<Self>,
    ) -> AnyElement {
        let params = &preset.params;
        let background = rgb_to_hsla(params.background);
        let luminance = 0.2126 * f32::from(params.background[0])
            + 0.7152 * f32::from(params.background[1])
            + 0.0722 * f32::from(params.background[2]);
        let ink = if luminance < 128.0 { white() } else { black() };
        // 这里的固定色是用户指定的“高斯模糊”预览图例，不是应用主题色。
        let canvas_background = if params.solid_background {
            background.into()
        } else {
            linear_gradient(
                135.0,
                linear_color_stop(rgba(0x42e695ff), 0.0),
                linear_color_stop(rgba(0x3bb2b8ff), 1.0),
            )
        };
        let canvas_ratio = params
            .aspect_ratio
            .map(|(width, height)| width / height.max(0.001))
            .unwrap_or(1.5)
            .clamp(0.5, 2.0) as f32;
        // 小尺寸图例里适度放大边框比例；零宽保持为零，避免产生并不存在的边框。
        let border_top = (params.border_ratio.0 as f32 * 3.2).clamp(0.0, 0.28);
        let border_bottom = (params.border_ratio.1 as f32 * 3.2).clamp(0.0, 0.28);
        let border_left = (params.border_ratio.2 as f32 * 3.2).clamp(0.0, 0.28);
        let border_right = (params.border_ratio.3 as f32 * 3.2).clamp(0.0, 0.28);
        h_flex()
            .w_full()
            .h_10()
            .items_center()
            .justify_end()
            .pr_1()
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .h_10()
                    .aspect_ratio(canvas_ratio)
                    .bg(canvas_background)
                    .border_1()
                    .border_color(ink.opacity(0.42))
                    .rounded(cx.theme().radius)
                    .when(params.shadow_size > 0.0, |this| this.shadow_sm())
                    .child(
                        div()
                            .absolute()
                            .top(relative(border_top))
                            .bottom(relative(border_bottom))
                            .left(relative(border_left))
                            .right(relative(border_right))
                            .bg(ink.opacity(0.24))
                            .border_1()
                            .border_color(ink.opacity(0.62))
                            .rounded(cx.theme().radius),
                    )
                    .children(
                        preset
                            .text_groups
                            .iter()
                            .map(|group| preset_text_marker(group, ink.opacity(0.88))),
                    ),
            )
            .into_any_element()
    }

    // MARK: 画布与边框

    fn render_canvas_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;
        let equal = self.params.border_equal;

        // 四个边框滑块收进一个可折叠区：不调边框的时候没必要占四行。
        let border_items = Accordion::new("border-width")
            .multiple(false)
            .bordered(false)
            // 组件默认铺满父级高度，在自动高度的分组里显式交回去。
            .h_auto()
            .item(|item| {
                item.title(hint(
                    format!(
                        "上 {:.1}% · 下 {:.1}% · 左 {:.1}% · 右 {:.1}%",
                        self.params.border_ratio.0 * 100.0,
                        self.params.border_ratio.1 * 100.0,
                        self.params.border_ratio.2 * 100.0,
                        self.params.border_ratio.3 * 100.0,
                    ),
                    cx,
                ))
                .open(self.border_width_open)
                .child(controls.border_top.render("上边框", false, cx))
                .child(controls.border_bottom.render("下边框", equal, cx))
                .child(controls.border_left.render("左边框", equal, cx))
                .child(controls.border_right.render("右边框", equal, cx))
            })
            .on_toggle_click(cx.listener(|this, open: &[usize], _, cx| {
                let is_open = !open.is_empty();
                if this.border_width_open != is_open {
                    this.border_width_open = is_open;
                    cx.notify();
                }
            }));

        GroupBox::new()
            .id("canvas-section")
            .title("画布与边框")
            .child(
                Switch::new("border-equal")
                    .checked(equal)
                    .label("四边等宽")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.params.border_equal = *checked;
                        this.refresh_preview(cx);
                        cx.notify();
                    })),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child("边框宽度"),
                    )
                    .child(border_items),
            )
            .child(field(
                "宽高比",
                Select::new(&controls.aspect_ratio).w_full(),
                cx,
            ))
            .when(self.aspect_choice == AspectRatioChoice::Custom, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&controls.aspect_width)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(":"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&controls.aspect_height)),
                        ),
                )
            })
            .when(self.aspect_choice == AspectRatioChoice::Free, |this| {
                this.child(hint("不限制：画布尺寸只跟照片和边框有关。", cx))
            })
            .child(field(
                "图片位置",
                Select::new(&controls.position).w_full(),
                cx,
            ))
    }

    // MARK: 背景

    fn render_background_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;
        let solid = self.params.solid_background;

        GroupBox::new()
            .id("background-section")
            .title("背景")
            .child(
                Switch::new("solid-background")
                    .checked(solid)
                    .label("纯色背景")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.params.solid_background = *checked;
                        this.refresh_preview(cx);
                        cx.notify();
                    })),
            )
            .child(controls.background.render("背景颜色", !solid, cx))
            .child(controls.blur_sigma.render("模糊强度", solid, cx))
    }

    // MARK: 图片细节

    fn render_image_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;

        GroupBox::new()
            .id("image-section")
            .title("图片细节")
            .child(controls.border_radius.render("圆角", false, cx))
            .child(controls.shadow_size.render("阴影大小", false, cx))
            .child(controls.shadow_density.render("阴影浓度", false, cx))
    }

    fn render_output_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;

        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&controls.output_folder)),
                    )
                    .child(
                        Button::new("output-folder-pick")
                            .icon(IconName::FolderOpen)
                            .outline()
                            .small()
                            .tooltip("选择输出文件夹")
                            .accessibility_label("选择输出文件夹")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_output_folder(window, cx)
                            })),
                    ),
            )
            .child(controls.quality.render("JPEG 质量", false, cx))
            .child(
                Button::new("rotate-watermark-photo")
                    .icon(IconName::RotateCw)
                    .label(format!(
                        "顺时针旋转 90° · 当前 {}°",
                        self.params.rotation.degrees()
                    ))
                    .outline()
                    .small()
                    .w_full()
                    .disabled(self.selected_photo().is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.params.rotation = this.params.rotation.next();
                        this.refresh_preview(cx);
                        cx.notify();
                    })),
            )
            .child(self.render_export_footer(cx))
    }

    /// 导出是这一页唯一的提交动作，所以它落在面板最底部，并且是唯一的 primary 按钮。
    fn render_export_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let total = self.photos().len();
        let status = match &self.export {
            ExportState::Running { completed, total } => format!("正在导出 {completed}/{total}"),
            ExportState::Finished {
                succeeded,
                failed: 0,
            } => format!("已导出 {succeeded} 张"),
            ExportState::Finished { succeeded, failed } => {
                format!("已导出 {succeeded} 张，{failed} 张失败")
            }
            ExportState::Idle => String::new(),
        };

        v_flex()
            .w_full()
            .gap_2()
            .child(
                Button::new("export")
                    .icon(IconName::FolderOpen)
                    .label("导出全部")
                    .primary()
                    .w_full()
                    .disabled(total == 0 || matches!(self.export, ExportState::Running { .. }))
                    .on_click(cx.listener(|this, _, window, cx| this.export_all(window, cx))),
            )
            .when(!status.is_empty(), |this| this.child(hint(status, cx)))
            .child(hint(
                match &self.params.output_folder {
                    Some(folder) => format!("导出到 {}", folder.display()),
                    None => "先填写输出文件夹".to_string(),
                },
                cx,
            ))
    }
}
