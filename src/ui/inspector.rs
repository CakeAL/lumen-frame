//! 右侧参数面板。
//!
//! 面板里的控件实体只保存控件自身的状态（滑块位置、下拉框开合、取色器面板），参数值
//! 始终以 [`AppView::params`] 为准：控件回调写入参数，然后请求一次预览重算。这样预览、
//! 导出、界面读数永远来自同一份数据。

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    accordion::Accordion,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::SelectEvent,
    select::{Select, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, FontWeight, IntoElement, SharedString, Subscription, Window, div};

use crate::Position;
use crate::params::WatermarkParams;
use crate::process::text::Text;

use super::field::{
    Choice, ColorField, NumberField, choices, field, hint, index_of, on_select, select_state,
    warning,
};
use super::{AppView, ExportState};

/// 模糊强度的上限。
///
/// 再往上 blur 的耗时涨得比效果快，实际也很难看出差别，所以界面上就收在 150。
const BLUR_MAX: f64 = 150.0;

/// 宽高比的选项：不限制、常用比例、自定义。
///
/// 「不动宽高比」和「指定一个比例」是两件事，所以不强制比例的选项叫「不限制」。
#[derive(Clone, PartialEq)]
pub(super) enum AspectRatioChoice {
    /// 不限制：画布尺寸只跟照片和边框有关。
    Free,
    Preset(f64, f64),
    /// 自定义比例，具体数值在旁边两个输入框里。
    Custom,
}

pub(super) const ASPECT_RATIOS: &[(&str, AspectRatioChoice)] = &[
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
pub(super) const POSITIONS: &[(&str, Position)] = &[
    ("居中", Position::Center),
    ("靠上", Position::Up),
    ("靠下", Position::Bottom),
    ("靠左", Position::Left),
    ("靠右", Position::Right),
];

/// 下拉框状态的具体类型别名，免得这串泛型在签名里反复出现。
pub(super) type AspectRatioSelect = SelectState<Vec<Choice<AspectRatioChoice>>>;
pub(super) type PositionSelect = SelectState<Vec<Choice<Position>>>;
pub(super) type TimeFormatSelect = SelectState<Vec<Choice<String>>>;

/// 面板里所有需要跨帧保留的控件状态。
pub(super) struct ParameterControls {
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
    pub text_position: Entity<PositionSelect>,

    pub time_format: Entity<InputState>,
    /// 常用时间格式：选一个例子就把它填进时间格式输入框。
    pub time_format_example: Entity<TimeFormatSelect>,
    pub output_folder: Entity<InputState>,
    pub preset_name: Entity<InputState>,
}

impl ParameterControls {
    /// 创建全部控件，并把「控件变化 → 写回参数 → 请求预览」这条链路一次接好。
    pub(super) fn new(
        params: &WatermarkParams,
        time_format: &str,
        text_position: crate::Position,
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
        let text_position = select_state(
            choices(POSITIONS),
            index_of(POSITIONS, &text_position),
            window,
            cx,
        );

        subscriptions.push(on_select(&aspect_ratio, window, cx, |this, choice, cx| {
            this.apply_aspect_choice(choice, cx);
        }));
        subscriptions.push(on_select(&position, window, cx, |this, position, _| {
            this.params.position = position;
        }));
        subscriptions.push(on_select(
            &text_position,
            window,
            cx,
            |this, position, _| {
                this.text_position = position;
            },
        ));

        // 自定义比例：两个框任意一个变了，就拿两个框当前的值重算比例。
        for input in [&aspect_width, &aspect_height] {
            subscriptions.push(cx.subscribe_in(input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_custom_aspect_ratio(cx);
                }
            }));
        }

        let time_format = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(time_format.to_owned())
                .placeholder("%Y/%m/%d")
        });
        subscriptions.push(
            cx.subscribe_in(&time_format, window, |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.time_format = state.read(cx).value().to_string();
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );

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
                    this.params.output_folder = if value.trim().is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(value))
                    };
                    cx.notify();
                }
            }),
        );

        let time_format_example =
            select_state(super::text_section::time_format_choices(), None, window, cx);
        subscriptions.push(cx.subscribe_in(&time_format_example, window, {
            let input = time_format.clone();
            move |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(template)) = event else {
                    return;
                };
                this.time_format = template.clone();
                input.update(cx, |state, cx| {
                    state.set_value(template.clone(), window, cx)
                });
                this.refresh_preview(cx);
                cx.notify();
            }
        }));

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
                text_position,
                time_format,
                time_format_example,
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
pub(super) fn preset_of(params: &WatermarkParams, text: &Text) -> crate::config::WatermarkPreset {
    crate::config::WatermarkPreset {
        params: params.clone(),
        text: text.clone(),
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
                    .child(self.render_preset_section(cx))
                    .child(self.render_canvas_section(cx))
                    .child(self.render_background_section(cx))
                    .child(self.render_image_section(cx))
                    .child(self.render_text_section(cx))
                    .child(self.render_output_section(cx)),
            )
    }

    // MARK: 预设

    fn render_preset_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let name_ready = !self.controls.preset_name.read(cx).value().trim().is_empty();

        GroupBox::new().id("preset-section").title("预设").child(
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
                                .child(Input::new(&self.controls.preset_name)),
                        )
                        .child(
                            Button::new("preset-save")
                                .label("保存")
                                .disabled(!name_ready)
                                .on_click(cx.listener(|this, _, _, cx| this.save_preset(cx))),
                        ),
                )
                .child(hint(
                    "预设保存参数与文字水印；输出文件夹属于本机设置，不写进预设。",
                    cx,
                ))
                .when_some(self.preset_feedback.clone(), |this, feedback| {
                    this.child(if self.preset_feedback_is_error {
                        warning(feedback, cx)
                    } else {
                        hint(feedback, cx)
                    })
                })
                .when(self.preset_names.is_empty(), |this| {
                    this.child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(hint("还没有保存过预设。", cx)),
                    )
                })
                .when(!self.preset_names.is_empty(), |this| {
                    this.child(
                        v_flex().w_full().gap_2().children(
                            self.preset_names
                                .iter()
                                .enumerate()
                                .map(|(ix, name)| self.render_preset_row(ix, name.clone(), cx)),
                        ),
                    )
                })
                .child(
                    Button::new("preset-reset")
                        .icon(IconName::Undo2)
                        .label("恢复默认参数")
                        .ghost()
                        .w_full()
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.confirm_reset_params(window, cx)
                            }),
                        ),
                ),
        )
    }

    fn render_preset_row(
        &self,
        ix: usize,
        name: SharedString,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let for_load = name.clone();
        let for_delete = name.clone();

        h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(name),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .child(
                        Button::new(("preset-load", ix))
                            .label("载入")
                            .small()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.load_preset(&for_load, window, cx)
                            })),
                    )
                    .child(
                        Button::new(("preset-delete", ix))
                            .icon(IconName::Delete)
                            .ghost()
                            .small()
                            .tooltip("删除这个预设")
                            .accessibility_label(format!("删除预设 {for_delete}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_preset(&for_delete, window, cx)
                            })),
                    ),
            )
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

    // MARK: 输出

    fn render_output_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;

        GroupBox::new()
            .id("output-section")
            .title("输出")
            .child(field("输出文件夹", Input::new(&controls.output_folder), cx))
            .child(controls.quality.render("JPEG 质量", false, cx))
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
                    .on_click(cx.listener(|this, _, _, cx| this.export_all(cx))),
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
