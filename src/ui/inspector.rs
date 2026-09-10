//! 右侧参数面板。
//!
//! 面板里的控件实体只保存控件自身的状态（滑块位置、下拉框开合、取色器面板），参数值
//! 始终以 [`AppView::params`] 为准：控件回调写入参数，然后请求一次预览重算。这样
//! 预览、导出、界面读数永远来自同一份数据。

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState},
    searchable_list::SearchableListItem,
    select::{Select, SelectEvent, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FontWeight, Hsla, Rgba, SharedString, Subscription, Window, div,
};

use crate::Position;
use crate::params::WatermarkParams;
use crate::process::text::{Text, TextAlign};

use super::AppView;
use super::ExportState;

/// 宽高比预设。`None` 表示跟随原图尺寸。
const ASPECT_RATIOS: &[(&str, Option<(f64, f64)>)] = &[
    ("跟随原图", None),
    ("1:1", Some((1.0, 1.0))),
    ("4:5", Some((4.0, 5.0))),
    ("5:4", Some((5.0, 4.0))),
    ("3:2", Some((3.0, 2.0))),
    ("2:3", Some((2.0, 3.0))),
    ("16:9", Some((16.0, 9.0))),
    ("9:16", Some((9.0, 16.0))),
];

/// 图片与文字水印可以贴的位置。
pub(super) const POSITIONS: &[(&str, Position)] = &[
    ("居中", Position::Center),
    ("靠上", Position::Up),
    ("靠下", Position::Bottom),
    ("靠左", Position::Left),
    ("靠右", Position::Right),
];

/// 文字对齐方式。
pub(super) const TEXT_ALIGNS: &[(&str, TextAlign)] = &[
    ("左对齐", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("右对齐", TextAlign::Right),
];

/// 给下拉框用的「标签 + 领域值」选项。
///
/// 直接把选项做成字符串的话，选中结果还要再靠文本反解回领域值；这里让下拉框把领域值
/// 本身带回来。
#[derive(Clone)]
pub(super) struct Choice<T: Clone + PartialEq + 'static> {
    label: SharedString,
    value: T,
}

impl<T: Clone + PartialEq + 'static> Choice<T> {
    pub(super) fn new(label: impl Into<SharedString>, value: T) -> Self {
        Self {
            label: label.into(),
            value,
        }
    }
}

impl<T: Clone + PartialEq + 'static> SearchableListItem for Choice<T> {
    type Value = T;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// 把领域值转成下拉框选项。
pub(super) fn choices<T: Clone + PartialEq + 'static>(entries: &[(&str, T)]) -> Vec<Choice<T>> {
    entries
        .iter()
        .map(|(label, value)| Choice::new(*label, value.clone()))
        .collect()
}

/// 宽高比下拉框的状态。抽成别名，免得这串泛型在签名里反复出现。
type AspectRatioSelect = SelectState<Vec<Choice<Option<(f64, f64)>>>>;

/// 面板里所有需要跨帧保留的控件状态。
pub(super) struct ParameterControls {
    pub border_top: Entity<SliderState>,
    pub border_bottom: Entity<SliderState>,
    pub border_left: Entity<SliderState>,
    pub border_right: Entity<SliderState>,
    pub border_radius: Entity<SliderState>,
    pub shadow_size: Entity<SliderState>,
    pub shadow_density: Entity<SliderState>,
    pub blur_sigma: Entity<SliderState>,
    pub quality: Entity<SliderState>,
    pub background_color: Entity<ColorPickerState>,
    pub aspect_ratio: Entity<AspectRatioSelect>,
    pub position: Entity<SelectState<Vec<Choice<Position>>>>,
    pub text_position: Entity<SelectState<Vec<Choice<Position>>>>,
    pub time_format: Entity<InputState>,
    pub output_folder: Entity<InputState>,
}

impl ParameterControls {
    /// 创建全部控件，并把「控件变化 → 写回参数 → 请求预览」这条链路一次接好。
    pub(super) fn new(
        params: &WatermarkParams,
        text: &Text,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> (Self, Vec<Subscription>) {
        let border_top = slider(params.border_ratio.0 as f32, 0.0, 0.4, 0.005, cx);
        let border_bottom = slider(params.border_ratio.1 as f32, 0.0, 0.4, 0.005, cx);
        let border_left = slider(params.border_ratio.2 as f32, 0.0, 0.4, 0.005, cx);
        let border_right = slider(params.border_ratio.3 as f32, 0.0, 0.4, 0.005, cx);
        let border_radius = slider(params.border_radius as f32, 0.0, 0.2, 0.002, cx);
        let shadow_size = slider(params.shadow_size as f32, 0.0, 0.3, 0.005, cx);
        let shadow_density = slider(params.shadow_density as f32, 0.0, 2.0, 0.05, cx);
        let blur_sigma = slider(params.blur_sigma as f32, 0.0, 1000.0, 5.0, cx);
        let quality = slider(params.quality as f32, 1.0, 100.0, 1.0, cx);

        let background_color = cx.new(|cx| {
            ColorPickerState::new(window, cx).default_value(rgb_to_hsla(params.background))
        });

        let aspect_ratio = cx.new(|cx| {
            let items = choices(ASPECT_RATIOS);
            let selected = ASPECT_RATIOS
                .iter()
                .position(|(_, value)| *value == params.aspect_ratio)
                .map(IndexPath::new);
            SelectState::new(items, selected, window, cx)
        });

        let position = cx.new(|cx| {
            let items = choices(POSITIONS);
            let selected = POSITIONS
                .iter()
                .position(|(_, value)| *value == params.position)
                .map(IndexPath::new);
            SelectState::new(items, selected, window, cx)
        });

        let text_position = cx.new(|cx| {
            let items = choices(POSITIONS);
            let selected = POSITIONS
                .iter()
                .position(|(_, value)| *value == text.position)
                .map(IndexPath::new);
            SelectState::new(items, selected, window, cx)
        });

        let time_format = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(text.time_format.clone())
                .placeholder("例如 %Y/%m/%d")
        });

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

        let mut subscriptions = Vec::new();

        subscriptions.push(apply_slider(&border_top, cx, |params, value| {
            params.border_ratio.0 = value as f64;
        }));
        subscriptions.push(apply_slider(&border_bottom, cx, |params, value| {
            params.border_ratio.1 = value as f64;
        }));
        subscriptions.push(apply_slider(&border_left, cx, |params, value| {
            params.border_ratio.2 = value as f64;
        }));
        subscriptions.push(apply_slider(&border_right, cx, |params, value| {
            params.border_ratio.3 = value as f64;
        }));
        subscriptions.push(apply_slider(&border_radius, cx, |params, value| {
            params.border_radius = value as f64;
        }));
        subscriptions.push(apply_slider(&shadow_size, cx, |params, value| {
            params.shadow_size = value as f64;
        }));
        subscriptions.push(apply_slider(&shadow_density, cx, |params, value| {
            params.shadow_density = value as f64;
        }));
        subscriptions.push(apply_slider(&blur_sigma, cx, |params, value| {
            params.blur_sigma = value as f64;
        }));
        subscriptions.push(apply_slider(&quality, cx, |params, value| {
            params.quality = value.round() as i32;
        }));

        subscriptions.push(cx.subscribe(&background_color, |this, _, event, cx| {
            if let ColorPickerEvent::Change(Some(color)) = event {
                this.params.background = hsla_to_rgb(*color);
                this.refresh_preview(cx);
                cx.notify();
            }
        }));

        subscriptions.push(cx.subscribe(&aspect_ratio, |this, _, event, cx| {
            let SelectEvent::Confirm(value) = event;
            // 未选中和「跟随原图」都表示不给宽高比约束。
            this.params.aspect_ratio = (*value).flatten();
            this.refresh_preview(cx);
            cx.notify();
        }));

        subscriptions.push(cx.subscribe(&position, |this, _, event, cx| {
            let SelectEvent::Confirm(value) = event;
            this.params.position = value.unwrap_or(Position::Center);
            this.refresh_preview(cx);
            cx.notify();
        }));

        subscriptions.push(cx.subscribe(&text_position, |this, _, event, cx| {
            let SelectEvent::Confirm(value) = event;
            this.text_position = value.unwrap_or(Position::Bottom);
            this.refresh_preview(cx);
            cx.notify();
        }));

        subscriptions.push(
            cx.subscribe_in(&time_format, window, |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.time_format = state.read(cx).value().to_string();
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );

        subscriptions.push(
            cx.subscribe_in(&output_folder, window, |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_string();
                    this.params.output_folder = if value.trim().is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(value))
                    };
                    cx.notify();
                }
            }),
        );

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
                background_color,
                aspect_ratio,
                position,
                text_position,
                time_format,
                output_folder,
            },
            subscriptions,
        )
    }
}

impl AppView {
    pub(super) fn render_inspector(&self, cx: &Context<Self>) -> impl IntoElement {
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
                    .py_2()
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
                    .child(self.render_text_section(cx))
                    .child(self.render_output_section(cx)),
            )
    }

    fn render_canvas_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;
        let equal = self.params.border_equal;

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
            .child(slider_row(
                "上边框",
                percent(slider_value(&controls.border_top, cx)),
                &controls.border_top,
                false,
                cx,
            ))
            .child(slider_row(
                "下边框",
                percent(slider_value(&controls.border_bottom, cx)),
                &controls.border_bottom,
                equal,
                cx,
            ))
            .child(slider_row(
                "左边框",
                percent(slider_value(&controls.border_left, cx)),
                &controls.border_left,
                equal,
                cx,
            ))
            .child(slider_row(
                "右边框",
                percent(slider_value(&controls.border_right, cx)),
                &controls.border_right,
                equal,
                cx,
            ))
            .child(field(
                "宽高比",
                Select::new(&controls.aspect_ratio).w_full(),
                cx,
            ))
            .child(field(
                "图片位置",
                Select::new(&controls.position).w_full(),
                cx,
            ))
    }

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
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(if solid {
                                cx.theme().foreground
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child("背景颜色"),
                    )
                    .child(ColorPicker::new(&controls.background_color)),
            )
            .child(slider_row(
                "模糊强度",
                format!("{:.0}", slider_value(&controls.blur_sigma, cx)),
                &controls.blur_sigma,
                solid,
                cx,
            ))
    }

    fn render_image_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;

        GroupBox::new()
            .id("image-section")
            .title("图片细节")
            .child(slider_row(
                "圆角",
                percent(slider_value(&controls.border_radius, cx)),
                &controls.border_radius,
                false,
                cx,
            ))
            .child(slider_row(
                "阴影大小",
                percent(slider_value(&controls.shadow_size, cx)),
                &controls.shadow_size,
                false,
                cx,
            ))
            .child(slider_row(
                "阴影浓度",
                format!("{:.2}", slider_value(&controls.shadow_density, cx)),
                &controls.shadow_density,
                false,
                cx,
            ))
    }

    fn render_output_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;

        GroupBox::new()
            .id("output-section")
            .title("输出")
            .child(field("输出文件夹", Input::new(&controls.output_folder), cx))
            .child(slider_row(
                "JPEG 质量",
                format!("{:.0}", slider_value(&controls.quality, cx)),
                &controls.quality,
                false,
                cx,
            ))
            .child(self.render_export_footer(cx))
    }

    /// 导出是这一页唯一的提交动作，所以它落在面板最底部，并且是唯一的 primary 按钮。
    fn render_export_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let total = self.photos.len();
        let status = match &self.export {
            ExportState::Running { completed, total } => {
                format!("正在导出 {completed}/{total}")
            }
            ExportState::Finished {
                succeeded,
                failed: 0,
            } => {
                format!("已导出 {succeeded} 张")
            }
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
                    .on_click(cx.listener(|this, _, _, cx| this.export_all(cx))),
            )
            .when(!status.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .child(hint(
                match &self.params.output_folder {
                    Some(folder) => format!("导出到 {}", folder.display()),
                    None => "先填写输出文件夹".to_string(),
                },
                cx,
            ))
    }
}

/// 一行「标签 + 读数 + 滑块」。
///
/// 读数右对齐、滑块占满整行，多个这样的行叠起来会形成稳定的纵向对齐，扫一眼就能比较
/// 各参数的相对大小。
pub(super) fn slider_row(
    label: impl Into<SharedString>,
    reading: String,
    slider: &Entity<SliderState>,
    disabled: bool,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .gap_2()
                .text_sm()
                .child(
                    div()
                        .text_color(if disabled {
                            cx.theme().muted_foreground
                        } else {
                            cx.theme().foreground
                        })
                        .child(label.into()),
                )
                .child(div().text_color(cx.theme().muted_foreground).child(reading)),
        )
        .child(Slider::new(slider).disabled(disabled).w_full())
}

/// 一行「标签在上、控件在下」的字段。
pub(super) fn field(
    label: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(label.into()),
        )
        .child(control)
}

/// 一段说明性文字，用于解释当前参数组合下的后果。
pub(super) fn hint(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub(super) fn slider_value(slider: &Entity<SliderState>, cx: &App) -> f32 {
    slider.read(cx).value().end()
}

pub(super) fn percent(value: f32) -> String {
    format!("{:.1}%", value * 100.0)
}

/// 建一个滑块实体。控件自身持有位置，参数值仍在 [`AppView::params`] 里。
fn slider(
    initial: f32,
    min: f32,
    max: f32,
    step: f32,
    cx: &mut Context<AppView>,
) -> Entity<SliderState> {
    cx.new(|_| {
        SliderState::new()
            .min(min)
            .max(max)
            .step(step)
            .default_value(initial)
    })
}

/// 把滑块变化写回参数字段。
fn apply_slider(
    slider: &Entity<SliderState>,
    cx: &mut Context<AppView>,
    apply: impl Fn(&mut WatermarkParams, f32) + 'static,
) -> Subscription {
    cx.subscribe(slider, move |this, _, event, cx| {
        if let SliderEvent::Change(value) = event {
            apply(&mut this.params, value.end());
            this.refresh_preview(cx);
        }
    })
}

pub(super) fn hsla_to_rgb(color: Hsla) -> [u8; 3] {
    let rgba: Rgba = color.into();
    [
        (rgba.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub(super) fn rgb_to_hsla(rgb: [u8; 3]) -> Hsla {
    Rgba {
        r: rgb[0] as f32 / 255.0,
        g: rgb[1] as f32 / 255.0,
        b: rgb[2] as f32 / 255.0,
        a: 1.0,
    }
    .into()
}
