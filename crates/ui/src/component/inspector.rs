//! 预设面板与右侧参数面板。
//!
//! 面板里的控件实体只保存控件自身的状态（滑块位置、下拉框开合、取色器面板），参数值
//! 始终以 [`AppView::params`] 为准：控件回调写入参数，然后请求一次预览重算。这样预览、
//! 导出、界面读数永远来自同一份数据。

use gpui_kit::TestSupportExt as _;
use gpui_kit::base::{
    Accordion as BaseAccordion, AccordionHeader, AccordionItem, AccordionPanel, AccordionTrigger,
    motion::{Transition, transition},
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    kbd::Kbd,
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, DragMoveEvent, Entity, EntityId, Focusable as _, FontWeight, Hsla,
    IntoElement, KeyDownEvent, Keystroke, Role, ScrollHandle, SharedString, Subscription, Window,
    black, div, linear_color_stop, linear_gradient, percentage, relative, rgba, white,
};

use crate::backend::watermark::{
    BackgroundGradient, BackgroundKind, PaletteTheme, Placement, ShadowDirection, TextAlign,
    TextDirection, TextGroup, WatermarkParams,
};

use super::super::{AppView, ExportState, PresetScope};
use super::field::{
    Choice, ColorField, NumberField, choices, description, field, index_of, muted_text, on_select,
    parameter_card, rgb_to_hsla, select_state, warning,
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
        .border_1()
        .border_color(cx.theme().transparent)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

/// 检查器仅持有导航、展开和滚动状态；参数仍由后端会话持有。
pub(in crate::app) struct InspectorState {
    tab: usize,
    expanded: [bool; 3],
    scroll: [ScrollHandle; 2],
}

#[derive(Clone, Copy)]
enum InspectorSection {
    Canvas,
    Photo,
    Background,
}

impl InspectorSection {
    fn index(self) -> usize {
        match self {
            Self::Canvas => 0,
            Self::Photo => 1,
            Self::Background => 2,
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Canvas => "inspector-canvas-toggle",
            Self::Photo => "inspector-photo-toggle",
            Self::Background => "inspector-background-toggle",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Canvas => "画布",
            Self::Photo => "照片外观",
            Self::Background => "背景",
        }
    }
}

impl Default for InspectorState {
    fn default() -> Self {
        Self {
            tab: 0,
            expanded: [true, false, false],
            scroll: [ScrollHandle::new(), ScrollHandle::new()],
        }
    }
}

/// 模糊强度的上限。
///
/// 再往上 blur 的耗时涨得比效果快，实际也很难看出差别，所以界面上就收在 150。
const BLUR_MAX: f64 = 150.0;

/// 拖动携带预设名与工作台身份，不能跨工作台修改另一份顺序。
#[derive(Clone)]
struct DraggedPreset {
    name: SharedString,
    owner: EntityId,
}

impl Render for DraggedPreset {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w(gpui_kit::rems(15.0))
            .gap_2()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_md()
            .child(Icon::new(gpui_kit::assets::IconName::GripVertical).small())
            .child(div().min_w_0().truncate().child(self.name.clone()))
    }
}

/// 宽高比的选项：不限制、常用比例、自定义。
///
/// 「不动宽高比」和「指定一个比例」是两件事，所以不强制比例的选项叫「不限制」。
#[derive(Clone, PartialEq)]
pub(in crate::app) enum AspectRatioChoice {
    /// 不限制：画布尺寸只跟照片和边框有关。
    Free,
    Preset(f64, f64),
    /// 自定义比例，具体数值在旁边两个输入框里。
    Custom,
}

pub(in crate::app) const ASPECT_RATIOS: &[(&str, AspectRatioChoice)] = &[
    ("不限制", AspectRatioChoice::Free),
    ("1:1", AspectRatioChoice::Preset(1.0, 1.0)),
    ("3:4", AspectRatioChoice::Preset(3.0, 4.0)),
    ("4:3", AspectRatioChoice::Preset(4.0, 3.0)),
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
pub(in crate::app) type AspectRatioSelect = SelectState<Vec<Choice<AspectRatioChoice>>>;

/// 面板里所有需要跨帧保留的控件状态。
pub(in crate::app) struct ParameterControls {
    pub border_top: NumberField,
    pub border_bottom: NumberField,
    pub border_left: NumberField,
    pub border_right: NumberField,
    pub border_radius: NumberField,
    pub photo_border_width: NumberField,
    pub photo_border_color: ColorField,
    pub shadow_size: NumberField,
    pub shadow_density: NumberField,
    pub shadow_offset: NumberField,
    pub shadow_scale: NumberField,
    pub blur_sigma: NumberField,
    pub grain_strength: NumberField,
    pub quality: NumberField,
    pub background: ColorField,

    pub aspect_ratio: Entity<AspectRatioSelect>,
    /// 自定义宽高比的两个输入框。
    pub aspect_width: Entity<InputState>,
    pub aspect_height: Entity<InputState>,
    pub output_folder: Entity<InputState>,
    pub preset_name: Entity<InputState>,
    pub custom_text: Entity<InputState>,
    pub gradient_angle: NumberField,
    pub palette_theme: Entity<SelectState<Vec<Choice<PaletteTheme>>>>,
    pub border_palette_theme: Entity<SelectState<Vec<Choice<PaletteTheme>>>>,
}

impl ParameterControls {
    /// 创建全部控件，并把「控件变化 → 写回参数 → 请求预览」这条链路一次接好。
    pub(in crate::app) fn new(
        params: &WatermarkParams,
        aspect_choice: &AspectRatioChoice,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> (Self, Vec<Subscription>) {
        let mut subscriptions = Vec::new();

        let border_top = NumberField::new(
            params.border_ratio.0,
            0.0,
            200.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        )
        .logarithmic(window, cx);
        let border_bottom = NumberField::new(
            params.border_ratio.1,
            0.0,
            200.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        )
        .logarithmic(window, cx);
        let border_left = NumberField::new(
            params.border_ratio.2,
            0.0,
            200.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        )
        .logarithmic(window, cx);
        let border_right = NumberField::new(
            params.border_ratio.3,
            0.0,
            200.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        )
        .logarithmic(window, cx);
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
        let photo_border_width = NumberField::new(
            params.photo_border.width(),
            0.1,
            5.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let photo_border_color = ColorField::new(params.photo_border.color(), window, cx);
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
            4.0,
            0.05,
            2,
            1.0,
            "",
            window,
            cx,
        );
        let shadow_offset = NumberField::new(
            params.shadow_offset,
            -10.0,
            10.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let shadow_scale = NumberField::new(
            params.shadow_scale,
            50.0,
            120.0,
            0.1,
            1,
            100.0,
            "%",
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
        let grain_strength = NumberField::new(
            params.grain_strength,
            0.0,
            100.0,
            1.0,
            0,
            100.0,
            "%",
            window,
            cx,
        );
        let background = ColorField::new(params.background, window, cx);
        let gradient_angle = NumberField::new(
            params.gradient.angle(),
            0.0,
            359.0,
            1.0,
            0,
            1.0,
            "°",
            window,
            cx,
        );
        subscriptions.extend(gradient_angle.subscribe(window, cx, |this, value| {
            this.backend.params_mut().gradient =
                BackgroundGradient::new(value, this.backend.params().gradient.colors().to_vec())
                    .expect("valid gradient angle");
        }));
        let themes = super::background::PALETTE_THEMES;
        let palette_theme = select_state(
            choices(themes),
            index_of(themes, &params.gradient_palette.theme()),
            window,
            cx,
        );
        subscriptions.push(cx.subscribe_in(
            &palette_theme,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(theme)) = event {
                    this.set_palette_theme(*theme, window, cx);
                }
            },
        ));

        let border_palette_theme = select_state(
            choices(themes),
            index_of(themes, &params.gradient_palette.theme()),
            window,
            cx,
        );
        subscriptions.push(cx.subscribe_in(
            &border_palette_theme,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(theme)) = event {
                    this.set_palette_theme(*theme, window, cx);
                }
            },
        ));
        subscriptions.extend(photo_border_width.subscribe(window, cx, |this, value| {
            this.backend.params_mut().photo_border.set_width(value);
        }));
        subscriptions.extend(photo_border_color.subscribe(window, cx, |this, rgb| {
            // 控件同步派生的色卡值时保留来源，仅手动改色才取消色卡绑定。
            if this.photo_border_display_color() != rgb {
                this.backend.params_mut().photo_border.set_color(rgb);
            }
        }));

        subscriptions.extend(border_top.subscribe(window, cx, |this, value| {
            this.backend.params_mut().border_ratio.0 = value;
        }));
        subscriptions.extend(border_bottom.subscribe(window, cx, |this, value| {
            this.backend.params_mut().border_ratio.1 = value;
        }));
        subscriptions.extend(border_left.subscribe(window, cx, |this, value| {
            this.backend.params_mut().border_ratio.2 = value;
        }));
        subscriptions.extend(border_right.subscribe(window, cx, |this, value| {
            this.backend.params_mut().border_ratio.3 = value;
        }));
        subscriptions.extend(border_radius.subscribe(window, cx, |this, value| {
            this.backend.params_mut().border_radius = value;
        }));
        subscriptions.extend(shadow_size.subscribe(window, cx, |this, value| {
            this.backend.params_mut().shadow_size = value;
        }));
        subscriptions.extend(shadow_density.subscribe(window, cx, |this, value| {
            this.backend.params_mut().shadow_density = value;
        }));
        subscriptions.extend(shadow_offset.subscribe(window, cx, |this, value| {
            this.backend.params_mut().shadow_offset = value;
        }));
        subscriptions.extend(shadow_scale.subscribe(window, cx, |this, value| {
            this.backend.params_mut().shadow_scale = value;
        }));
        subscriptions.extend(blur_sigma.subscribe(window, cx, |this, value| {
            this.backend.params_mut().blur_sigma = value;
        }));
        subscriptions.extend(grain_strength.subscribe(window, cx, |this, value| {
            this.backend.params_mut().grain_strength = value;
        }));
        subscriptions.extend(quality.subscribe(window, cx, |this, value| {
            this.set_jpeg_quality(value.round() as i32);
        }));
        subscriptions.extend(background.subscribe(window, cx, |this, rgb| {
            this.backend.params_mut().background = rgb;
            this.backend.params_mut().background_palette_rank = None;
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
        subscriptions.push(on_select(&aspect_ratio, window, cx, |this, choice, cx| {
            this.apply_aspect_choice(choice, cx);
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

        let custom_text = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(params.custom_text.clone())
                .placeholder("当前图片的自定义文本")
        });
        subscriptions.push(
            cx.subscribe_in(&custom_text, window, |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) && this.selected_photo_id().is_some() {
                    let value = state.read(cx).value().to_string();
                    if value != this.backend.params().custom_text {
                        this.backend.params_mut().custom_text = value;
                        this.refresh_preview(cx);
                        cx.notify();
                    }
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
                photo_border_width,
                photo_border_color,
                shadow_size,
                shadow_density,
                shadow_offset,
                shadow_scale,
                blur_sigma,
                grain_strength,
                quality,
                background,
                gradient_angle,
                palette_theme,
                border_palette_theme,
                aspect_ratio,
                aspect_width,
                aspect_height,
                output_folder,
                preset_name,
                custom_text,
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
pub(in crate::app) fn preset_of(
    params: &WatermarkParams,
    text_groups: &[TextGroup],
) -> crate::backend::presets::WatermarkPreset {
    crate::backend::presets::WatermarkPreset {
        params: params.clone(),
        text_groups: text_groups.to_vec(),
    }
}

impl AppView {
    /// JPEG 质量属于本机导出偏好；所有照片在生成导出快照时读取当前值。
    fn set_jpeg_quality(&mut self, quality: i32) {
        let quality = quality.clamp(1, 100);
        if self.backend.params().quality != quality {
            self.backend.params_mut().quality = quality;
            self.persist_settings();
        }
    }

    pub(in crate::app) fn render_inspector(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.selected_photo();
        let photo_name = selected
            .and_then(|photo| photo.path().file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "新照片默认样式".to_owned());
        v_flex()
            .h_full()
            .min_h_0()
            .w_80()
            .flex_shrink_0()
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
                            .flex_1()
                            .child("参数"),
                    )
                    .child(
                        Button::new("preset-reset")
                            .icon(IconName::Undo2)
                            .ghost()
                            .small()
                            .tooltip("恢复默认参数…")
                            .accessibility_label("恢复默认参数…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_reset_params(window, cx)
                            })),
                    )
                    .child(
                        Button::new("apply-settings-to-all")
                            .label("应用到全部")
                            .small()
                            .outline()
                            .tooltip("将当前样式应用到所有照片，保留各照片的自定义文本")
                            .accessibility_label("应用当前设置到所有照片")
                            .disabled(selected.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_current_settings_to_all(window, cx)
                            })),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_shrink_0()
                    .gap_3()
                    .p_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().truncate().child(photo_name))
                            .child(description(if selected.is_some() {
                                "修改仅作用于当前照片"
                            } else {
                                "当前样式用于后续导入的照片"
                            })),
                    )
                    .child(parameter_card(
                        "inspector-custom-text-card",
                        field(
                            "自定义文本",
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(
                                    Input::new(&self.controls.custom_text)
                                        .id("photo-custom-text")
                                        .aria_label("当前图片的自定义文本")
                                        .disabled(selected.is_none())
                                        .flex_1()
                                        .min_w_0(),
                                )
                                .child(
                                    Button::new("photo-custom-text-next")
                                        .icon(IconName::ChevronRight)
                                        .size_8()
                                        .p_0()
                                        .tooltip("下一张照片")
                                        .accessibility_label("下一张照片")
                                        .disabled(
                                            selected.is_none()
                                                || self.photos().last().map(|photo| photo.id())
                                                    == self.selected_photo_id(),
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.select_adjacent_photo(false, window, cx);
                                            this.controls.custom_text.update(cx, |input, cx| {
                                                window.focus(&input.focus_handle(cx), cx)
                                            });
                                        })),
                                ),
                            cx,
                        ),
                    ))
                    .child(
                        TabBar::new("inspector-tabs")
                            .segmented()
                            .w_full()
                            .child(Tab::new().label("画面").flex_1())
                            .child(Tab::new().label("水印").flex_1())
                            .selected_index(self.inspector.tab)
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.inspector.tab = *ix;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id(if self.inspector.tab == 0 {
                        "inspector-image-scroll"
                    } else {
                        "inspector-watermark-scroll"
                    })
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.inspector.scroll[self.inspector.tab])
                    .vertical_scrollbar(&self.inspector.scroll[self.inspector.tab])
                    .child(
                        v_flex()
                            .w_full()
                            .gap_4()
                            .px_3()
                            .pb_4()
                            .when(self.inspector.tab == 0, |this| {
                                let params = self.backend.params();
                                let ratio = params
                                    .aspect_ratio
                                    .map(|(w, h)| {
                                        format!("{}:{}", format_number(w), format_number(h))
                                    })
                                    .unwrap_or_else(|| "不限比例".into());
                                let canvas = format!(
                                    "{ratio} · 留白 {:.1}/{:.1}/{:.1}/{:.1}%",
                                    params.border_ratio.0 * 100.,
                                    params.border_ratio.1 * 100.,
                                    params.border_ratio.2 * 100.,
                                    params.border_ratio.3 * 100.
                                );
                                let photo = format!(
                                    "描边{} · 圆角 {:.1}% · 阴影{}",
                                    if params.photo_border.enabled() {
                                        "开启"
                                    } else {
                                        "关闭"
                                    },
                                    params.border_radius * 100.,
                                    if params.shadow_size > 0. {
                                        "开启"
                                    } else {
                                        "关闭"
                                    }
                                );
                                let background = match params.background_kind() {
                                    BackgroundKind::Photo => {
                                        format!("照片模糊 {}", format_number(params.blur_sigma))
                                    }
                                    BackgroundKind::Solid => "纯色".into(),
                                    BackgroundKind::Gradient => {
                                        format!("渐变 · {} 色", params.gradient.colors().len())
                                    }
                                };
                                let background = format!(
                                    "{background} · 颗粒{}",
                                    if params.background_grain {
                                        "开启"
                                    } else {
                                        "关闭"
                                    }
                                );
                                this.child(self.render_inspector_section(
                                    InspectorSection::Canvas,
                                    canvas,
                                    self.render_canvas_section(cx).into_any_element(),
                                    window,
                                    cx,
                                ))
                                .child(self.render_inspector_section(
                                    InspectorSection::Photo,
                                    photo,
                                    self.render_image_section(cx).into_any_element(),
                                    window,
                                    cx,
                                ))
                                .child(
                                    self.render_inspector_section(
                                        InspectorSection::Background,
                                        background,
                                        self.render_background_section(cx).into_any_element(),
                                        window,
                                        cx,
                                    ),
                                )
                            })
                            .when(self.inspector.tab == 1, |this| {
                                this.child(self.render_text_section(cx))
                                    .child(parameter_card(
                                        "inspector-palette-card",
                                        self.render_palette_card_section(cx),
                                    ))
                            }),
                    ),
            )
    }

    fn render_inspector_section(
        &self,
        section: InspectorSection,
        summary: String,
        content: impl IntoElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ix = section.index();
        let title = section.title();
        // 箭头动效仍由框架处理反向切换、减少动态效果和帧调度。
        let progress = transition(
            (section.id(), "disclosure"),
            if self.inspector.expanded[ix] {
                1_f32
            } else {
                0_f32
            },
            Transition::new(cx.theme().motion_tokens().duration_normal),
            window,
            cx,
        );
        BaseAccordion::new(("inspector-section", ix))
            .w_full()
            .flex_shrink_0()
            .child(
                AccordionItem::new()
                    .open(self.inspector.expanded[ix])
                    .header(AccordionHeader::new(
                        AccordionTrigger::new(section.id())
                            .aria_label(title)
                            .flex()
                            .flex_row()
                            .items_center()
                            .w_full()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().secondary)
                            .text_color(cx.theme().secondary_foreground)
                            .focusable()
                            .tab_index(0)
                            .border_1()
                            .border_color(cx.theme().border)
                            .hover(|this| this.bg(cx.theme().secondary_hover))
                            .focus_visible(|this| this.border_color(cx.theme().ring))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .truncate()
                                            .child(summary),
                                    ),
                            )
                            .child(
                                Icon::new(IconName::ChevronRight)
                                    .xsmall()
                                    .rotate(percentage(progress * 0.25)),
                            )
                            .on_change({
                                let listener = cx.listener(move |this, open: &bool, _, cx| {
                                    this.inspector.expanded[ix] = *open;
                                    cx.notify();
                                });
                                move |open, _, window, cx| listener(&open, window, cx)
                            }),
                    ))
                    .panel(
                        AccordionPanel::new()
                            .id(("inspector-panel", ix))
                            // gpui-kit 0.8.1 的 MotionReveal 仍在 prepaint 中布局子树，
                            // 受控 Popover 此时移交焦点会触发辅助功能的重复焦点断言。
                            // 用普通布局让焦点在 request_layout 阶段确定，详见 docs/issues。
                            .child(v_flex().w_full().gap_3().pt_2().child(content)),
                    ),
            )
    }

    // MARK: 预设

    /// 左侧预设面板：两列卡片在独立滚动区内，保存与打开文件夹固定在顶部。
    pub(in crate::app) fn render_preset_panel(&self, cx: &Context<Self>) -> impl IntoElement {
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
                    .px_4()
                    .pt_4()
                    .pb_2()
                    .child(field(
                        "预设应用范围",
                        TabBar::new("preset-scope")
                            .segmented()
                            .w_full()
                            .children([
                                Tab::new().label("当前照片").flex_1(),
                                Tab::new().label("全部照片").flex_1(),
                            ])
                            .selected_index(if self.preset_scope == PresetScope::AllPhotos {
                                1
                            } else {
                                0
                            })
                            .on_click(cx.listener(|this, index: &usize, _, cx| {
                                this.preset_scope = if *index == 1 {
                                    PresetScope::AllPhotos
                                } else {
                                    PresetScope::CurrentPhoto
                                };
                                this.preset_feedback = None;
                                cx.notify();
                            })),
                        cx,
                    ))
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
                            muted_text(feedback, cx)
                        })
                    }),
            )
            .child(
                v_flex()
                    .id("preset-cards")
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .px_4()
                    .pt_2()
                    .pb_4()
                    .overflow_y_scroll()
                    .child(
                        BaseAccordion::new("builtin-presets")
                            .w_full()
                            .flex_shrink_0()
                            .child(
                                AccordionItem::new()
                                    .open(!self.builtin_presets_collapsed)
                                    // GPUI 的 Button 角色已将 Enter/Space 转为点击，统一走 on_change。
                                    .header(AccordionHeader::new(
                                        AccordionTrigger::new("builtin-presets-toggle")
                                            .aria_label("内置预设示例")
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .w_full()
                                            .justify_between()
                                            .py_1p5()
                                            .rounded(cx.theme().radius)
                                            .font_weight(FontWeight::MEDIUM)
                                            .border_1()
                                            .border_color(cx.theme().transparent)
                                            .focusable()
                                            .tab_index(0)
                                            .hover(|this| this.bg(cx.theme().muted))
                                            .focus_visible(|this| {
                                                this.border_color(cx.theme().ring)
                                            })
                                            .child(muted_text("内置预设示例", cx))
                                            .child(
                                                Icon::new(if self.builtin_presets_collapsed {
                                                    IconName::ChevronRight
                                                } else {
                                                    IconName::ChevronDown
                                                })
                                                .xsmall()
                                                .text_color(cx.theme().muted_foreground),
                                            )
                                            .on_change({
                                                let on_change =
                                                    cx.listener(|this, open: &bool, _, cx| {
                                                        this.set_builtin_presets_collapsed(
                                                            !open, cx,
                                                        );
                                                    });
                                                move |open, _, window, cx| {
                                                    on_change(&open, window, cx)
                                                }
                                            }),
                                    ))
                                    .panel(
                                        AccordionPanel::new().child(
                                            v_flex().pt_3().gap_3().children(
                                                self.preset_names
                                                    .iter()
                                                    .filter(|name| {
                                                        self.builtin_preset_names.contains(*name)
                                                    })
                                                    .map(|name| {
                                                        self.render_preset_card(
                                                            name.clone(),
                                                            true,
                                                            cx,
                                                        )
                                                    }),
                                            ),
                                        ),
                                    ),
                            ),
                    )
                    .when(
                        self.preset_names
                            .iter()
                            .any(|name| !self.builtin_preset_names.contains(name)),
                        |this| {
                            this.child(preset_section_label("自定义预设", cx)).child(
                                h_flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .child(description("拖动卡片排序，或按"))
                                    .child(Kbd::new(
                                        Keystroke::parse("alt-up").expect("有效的排序快捷键"),
                                    ))
                                    .child(Kbd::new(
                                        Keystroke::parse("alt-down").expect("有效的排序快捷键"),
                                    )),
                            )
                        },
                    )
                    .children(
                        self.preset_names
                            .iter()
                            .filter(|name| !self.builtin_preset_names.contains(*name))
                            .map(|name| self.render_preset_card(name.clone(), false, cx)),
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
        let card_id = format!("preset-card-{name}");
        let for_move = name.clone();
        let for_drop = name.clone();
        let for_drop_style = name.clone();
        let for_can_drop = name.clone();
        let owner = cx.entity_id();
        let drag_owner = cx.entity().downgrade();
        let dragged = DraggedPreset {
            name: name.clone(),
            owner,
        };
        let drop_before = self
            .preset_drop_target
            .as_ref()
            .filter(|(target, _)| target == &name)
            .map(|(_, before)| *before)
            .unwrap_or(true);
        let preview = self
            .preset_previews
            .get(&name)
            .map(|preset| self.render_preset_preview(preset, cx))
            .unwrap_or_else(|| muted_text("预览不可用", cx));

        v_flex()
            .relative()
            .w_full()
            .min_w_0()
            .child(
                h_flex()
                    .items_stretch()
                    .id(card_id.clone())
                    .debug_selector(move || card_id)
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
                        if !builtin
                            && event.keystroke.modifiers.alt
                            && matches!(event.keystroke.key.as_str(), "up" | "down")
                        {
                            cx.stop_propagation();
                            this.move_preset_by(&for_keyboard, event.keystroke.key == "up", cx);
                            return;
                        }
                        if event.keystroke.key == "escape" && cx.stop_active_drag(window) {
                            this.preset_drop_target = None;
                            cx.notify();
                        }
                    }))
                    .when(!builtin, |card| {
                        card.on_drag(dragged, move |dragged, _, _, cx| {
                            let _ = drag_owner.update(cx, |this, cx| {
                                this.preset_drop_target = None;
                                cx.notify();
                            });
                            cx.new(|_| dragged.clone())
                        })
                        .can_drop(move |value, _, _| {
                            value
                                .downcast_ref::<DraggedPreset>()
                                .is_some_and(|dragged| {
                                    dragged.owner == owner && dragged.name != for_can_drop
                                })
                        })
                        .drag_over::<DraggedPreset>(move |style, dragged, _, cx| {
                            if dragged.owner != owner || dragged.name == for_drop_style {
                                return style;
                            }
                            if drop_before {
                                style.border_t_2().border_color(cx.theme().primary)
                            } else {
                                style.border_b_2().border_color(cx.theme().primary)
                            }
                        })
                        .on_drag_move(cx.listener(
                            move |this, event: &DragMoveEvent<DraggedPreset>, _, cx| {
                                let dragged = event.drag(cx);
                                if dragged.owner != cx.entity_id() {
                                    return;
                                }
                                if event.bounds.contains(&event.event.position)
                                    && dragged.name != for_move
                                {
                                    let target = (
                                        for_move.clone(),
                                        event.event.position.y < event.bounds.center().y,
                                    );
                                    if this.preset_drop_target.as_ref() != Some(&target) {
                                        this.preset_drop_target = Some(target);
                                        cx.notify();
                                    }
                                } else if this
                                    .preset_drop_target
                                    .as_ref()
                                    .is_some_and(|(target, _)| target == &for_move)
                                {
                                    this.preset_drop_target = None;
                                    cx.notify();
                                }
                            },
                        ))
                        .on_drop(cx.listener(
                            move |this, dragged: &DraggedPreset, _, cx| {
                                if dragged.owner != cx.entity_id() {
                                    return;
                                }
                                if let Some((target, before)) = this.preset_drop_target.take()
                                    && target == for_drop
                                {
                                    cx.stop_propagation();
                                    this.move_preset(&dragged.name, &target, before, cx);
                                }
                            },
                        ))
                    })
                    .child(
                        v_flex()
                            .w_24()
                            .flex_shrink_0()
                            .min_w_0()
                            .justify_center()
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .line_clamp(2)
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child(name),
                            ),
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
        preset: &crate::backend::presets::WatermarkPreset,
        cx: &Context<Self>,
    ) -> AnyElement {
        let params = &preset.params;
        let rgb = params.text_background_color().unwrap_or(params.background);
        let background = rgb_to_hsla(rgb);
        let luminance =
            0.2126 * f32::from(rgb[0]) + 0.7152 * f32::from(rgb[1]) + 0.0722 * f32::from(rgb[2]);
        let ink = if luminance < 128.0 { white() } else { black() };
        // 这里的固定色是用户指定的“高斯模糊”预览图例，不是应用主题色。
        let canvas_background = if params.gradient_background {
            linear_gradient(
                params.gradient.angle() as f32,
                linear_color_stop(rgb_to_hsla(params.gradient.color_at(0.0)), 0.0),
                linear_color_stop(rgb_to_hsla(params.gradient.color_at(1.0)), 1.0),
            )
        } else if params.solid_background {
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
        let equal = self.backend.params().border_equal;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(parameter_card(
                "inspector-border-card",
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        Switch::new("border-equal")
                            .checked(equal)
                            .label("四边等宽")
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.backend.params_mut().border_equal = *checked;
                                this.refresh_preview(cx);
                                cx.notify();
                            })),
                    )
                    .when(equal, |this| {
                        this.child(controls.border_top.render("四边留白", false, cx))
                    })
                    .when(!equal, |this| {
                        this.child(controls.border_top.render("上留白", false, cx))
                            .child(controls.border_bottom.render("下留白", false, cx))
                            .child(controls.border_left.render("左留白", false, cx))
                            .child(controls.border_right.render("右留白", false, cx))
                    }),
            ))
            .child(parameter_card(
                "inspector-aspect-card",
                v_flex()
                    .w_full()
                    .gap_3()
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
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&controls.aspect_width)),
                                )
                                .child(div().text_sm().child(":"))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&controls.aspect_height)),
                                ),
                        )
                    }),
            ))
            .child(parameter_card(
                "inspector-position-card",
                v_flex().w_full().gap_3().child(field(
                    "照片位置",
                    TabBar::new("photo-position")
                        .segmented()
                        .w_full()
                        .children(
                            POSITIONS
                                .iter()
                                .map(|(label, _)| Tab::new().label(*label).flex_1()),
                        )
                        .selected_index(
                            POSITIONS
                                .iter()
                                .position(|(_, value)| *value == self.backend.params().position)
                                .unwrap_or(0),
                        )
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.backend.params_mut().position = POSITIONS[*ix].1;
                            this.refresh_preview(cx);
                            cx.notify();
                        })),
                    cx,
                )),
            ))
    }

    fn render_image_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.controls;
        v_flex()
            .w_full()
            .gap_3()
            .child(parameter_card(
                "inspector-radius-card",
                controls.border_radius.render("圆角", false, cx),
            ))
            .child(parameter_card(
                "inspector-stroke-card",
                self.render_photo_border_section(cx),
            ))
            .child(parameter_card(
                "inspector-shadow-blur-card",
                controls.shadow_size.render("阴影模糊", false, cx),
            ))
            .when(self.backend.params().shadow_size > 0., |this| {
                this.child(parameter_card(
                    "shadow-scale",
                    controls.shadow_scale.render("阴影大小", false, cx),
                ))
                .child(parameter_card(
                    "shadow-density",
                    controls.shadow_density.render("阴影浓度", false, cx),
                ))
                .child(parameter_card(
                    "inspector-shadow-color-card",
                    field(
                        "阴影颜色",
                        TabBar::new("shadow-color")
                            .segmented()
                            .w_full()
                            .child(Tab::new().label("黑色").flex_1())
                            .child(Tab::new().label("白色").flex_1())
                            .selected_index(usize::from(
                                self.backend.params().shadow_color == [255; 3],
                            ))
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.backend.params_mut().shadow_color =
                                    if *ix == 1 { [255; 3] } else { [0; 3] };
                                this.refresh_preview(cx);
                                cx.notify();
                            })),
                        cx,
                    ),
                ))
                .child(parameter_card(
                    "inspector-shadow-offset-card",
                    self.render_shadow_offset(cx),
                ))
            })
    }

    fn render_shadow_offset(&self, cx: &Context<Self>) -> impl IntoElement {
        const DIRECTIONS: &[(&str, &str, ShadowDirection)] = &[
            ("shadow-up-left", "左上", ShadowDirection::UpLeft),
            ("shadow-up", "上", ShadowDirection::Up),
            ("shadow-up-right", "右上", ShadowDirection::UpRight),
            ("shadow-left", "左", ShadowDirection::Left),
            ("shadow-center", "居中", ShadowDirection::Center),
            ("shadow-right", "右", ShadowDirection::Right),
            ("shadow-down-left", "左下", ShadowDirection::DownLeft),
            ("shadow-down", "下", ShadowDirection::Down),
            ("shadow-down-right", "右下", ShadowDirection::DownRight),
        ];
        let params = self.backend.params();
        v_flex()
            .w_full()
            .gap_3()
            .child(field(
                "偏移方向",
                v_flex()
                    .w_full()
                    .gap_1()
                    .children(DIRECTIONS.chunks(3).map(|row| {
                        h_flex().w_full().gap_1().children(row.iter().map(
                            |&(id, label, direction)| {
                                Button::new(id)
                                    .label(label)
                                    .small()
                                    .flex_1()
                                    .outline()
                                    .selected(params.shadow_direction == direction)
                                    .accessibility_label(format!("阴影偏移：{label}"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.backend.params_mut().shadow_direction = direction;
                                        this.refresh_preview(cx);
                                        cx.notify();
                                    }))
                            },
                        ))
                    })),
                cx,
            ))
            .child(
                div()
                    .id("shadow-offset")
                    .child(self.controls.shadow_offset.render("偏移量", false, cx)),
            )
            .child(description(
                "按照片高度计算，负值反向偏移；居中时不偏移。阴影大小低于 100% 可收拢轮廓。",
            ))
    }

    /// 导出是整个工作区的操作，固定在三栏下方，不随预设或参数滚动。
    pub(in crate::app) fn render_export_bar(&self, cx: &Context<Self>) -> impl IntoElement {
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
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(div().text_sm().flex_shrink_0().child("导出到"))
                            .child(
                                Input::new(&self.controls.output_folder)
                                    .aria_label("输出文件夹")
                                    .small()
                                    .flex_1()
                                    .min_w_0(),
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
                    .child(
                        div()
                            .id("jpeg-quality")
                            .w_48()
                            .map(|this| {
                                #[cfg(test)]
                                let this = this.test_support();
                                this
                            })
                            .child(self.controls.quality.render_compact("JPEG 质量", false, cx)),
                    )
                    .child(
                        Button::new("rotate-watermark-photo")
                            .icon(IconName::RotateCw)
                            .label(format!(
                                "旋转 {}°",
                                self.backend.params().rotation.degrees()
                            ))
                            .outline()
                            .small()
                            .tooltip("顺时针旋转全部导出照片，预览同步更新")
                            .disabled(self.selected_photo().is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.backend.params_mut().rotation =
                                    this.backend.params().rotation.next();
                                this.refresh_preview(cx);
                                cx.notify();
                            })),
                    )
                    .child(div().w_64().child(self.render_export_footer(cx))),
            )
            .when(!status.is_empty(), |this| {
                this.child(muted_text(status, cx))
            })
            .when(self.backend.params().output_folder.is_none(), |this| {
                this.child(description("先填写输出文件夹"))
            })
    }

    /// 当前照片与全部照片是同一提交动作的两种范围，批量导出保留 primary 强调。
    fn render_export_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let total = self.photos().len();
        let unavailable = self.backend.params().output_folder.is_none()
            || matches!(self.export, ExportState::Running { .. });
        h_flex()
            .w_full()
            .gap_2()
            .child(
                Button::new("export-current")
                    .icon(gpui_kit::assets::IconName::Download)
                    .label("导出当前")
                    .outline()
                    .small()
                    .flex_1()
                    .min_w_0()
                    .disabled(unavailable || self.selected_photo_id().is_none())
                    .tooltip("仅导出当前选中的照片")
                    .on_click(cx.listener(|this, _, window, cx| this.export_current(window, cx))),
            )
            .child(
                Button::new("export")
                    .icon(IconName::FolderOpen)
                    .label("导出全部")
                    .primary()
                    .small()
                    .flex_1()
                    .min_w_0()
                    .disabled(unavailable || total == 0 || self.photo_import.pending > 0)
                    .on_click(cx.listener(|this, _, window, cx| this.export_all(window, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, component::Root, test::TestWindowExt as _};

    struct InspectorHarness {
        app: Entity<AppView>,
        _subscription: Subscription,
    }
    impl Render for InspectorHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.app.update(cx, |app, cx| {
                app.render_inspector(window, cx).into_any_element()
            })
        }
    }

    #[gpui_kit::test]
    fn inspector_controls_leave_room_for_focus_and_expanded_content(cx: &mut TestAppContext) {
        use gpui_kit::{px, size};
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = AppView::new_with_settings_path(None, window, cx);
                app.inspector.expanded = [false, true, false];
                app
            });
            let panel = cx.new(|cx| InspectorHarness {
                _subscription: cx.observe(&app, |_, _, cx| cx.notify()),
                app,
            });
            Root::new(panel, window, cx)
        });
        for font_size in [14., 16., 18.] {
            cx.update(|_, cx| {
                gpui_kit::component::Theme::update(cx, |theme| theme.font_size = px(font_size))
            });
            cx.simulate_resize(size(px(20. * font_size), px(1800.)));
            cx.run_until_parked();
            cx.update(|window, cx| window.render_frame(cx));
            for _ in 0..2 {
                cx.update(|window, cx| window.click("photo-border-enabled", cx));
                cx.run_until_parked();
                cx.update(|window, cx| window.render_frame(cx));
                cx.update(|window, _| {
                    let panel = window.find(("inspector-panel", 1_usize)).bounds();
                    let switch = window.find("photo-border-enabled").bounds();
                    let card = window.find("inspector-stroke-card").bounds();
                    let following = window.find("inspector-shadow-blur-card").bounds();
                    assert!(panel.size.height > px(0.));
                    assert!(
                        switch.origin.x - panel.origin.x >= px(4.),
                        "开关焦点光圈需要留在展开面板的裁切边界内：{panel:?}, {switch:?}"
                    );
                    assert!(panel.right() - switch.right() >= px(4.));
                    assert!(
                        following.origin.y >= card.bottom() + px(8.),
                        "展开描边选项不能挤压或遮住下一张卡片"
                    );
                    assert!((following.origin.x - card.origin.x).abs() <= px(1.));
                    assert!((following.right() - card.right()).abs() <= px(1.));
                });
            }
        }
        cx.simulate_resize(size(px(320.), px(680.)));
        cx.run_until_parked();
        cx.update(|window, _| {
            assert!(window.find("inspector-image-scroll").bounds().size.height > px(100.));
        });
    }

    struct PopoverFocusHarness {
        app: Entity<AppView>,
        open: bool,
        trigger_focus: gpui_kit::FocusHandle,
        input: Entity<InputState>,
        phases: std::rc::Rc<std::cell::RefCell<Vec<Option<gpui_kit::FocusHandle>>>>,
    }
    impl Render for PopoverFocusHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let popup = gpui_kit::component::popover::Popover::new("focus-test-popup")
                .open(self.open)
                .trigger(Button::new("focus-test-popup-trigger").label("取色选项"))
                .child(Input::new(&self.input));
            let panel = self.app.update(cx, |app, cx| {
                app.render_inspector_section(
                    InspectorSection::Canvas,
                    String::new(),
                    popup,
                    window,
                    cx,
                )
                .into_any_element()
            });
            let before = self.phases.clone();
            let after = self.phases.clone();
            v_flex()
                .size_full()
                .child(
                    gpui_kit::canvas(
                        move |_, window, cx| before.borrow_mut().push(window.focused(cx)),
                        |_, _, _, _| {},
                    )
                    .h_1(),
                )
                .child(
                    div()
                        .id("focus-test-controller")
                        .role(Role::Button)
                        .track_focus(&self.trigger_focus)
                        .child("切换分组"),
                )
                .child(panel)
                .child(
                    gpui_kit::canvas(
                        move |_, window, cx| after.borrow_mut().push(window.focused(cx)),
                        |_, _, _, _| {},
                    )
                    .h_1(),
                )
        }
    }

    #[gpui_kit::test]
    fn inspector_popover_focus_is_set_before_prepaint(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let phases = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let (root, cx) = cx.add_window_view({
            let phases = phases.clone();
            move |window, cx| {
                let app = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                let harness = cx.new(|cx| PopoverFocusHarness {
                    app,
                    open: false,
                    trigger_focus: cx.focus_handle(),
                    input: cx.new(|cx| InputState::new(window, cx)),
                    phases,
                });
                Root::new(harness, window, cx)
            }
        });
        cx.run_until_parked();
        let harness = root.read_with(cx, |root, _| {
            root.view()
                .clone()
                .downcast::<PopoverFocusHarness>()
                .ok()
                .unwrap()
        });
        harness.update_in(cx, |harness, window, cx| {
            phases.borrow_mut().clear();
            harness.trigger_focus.focus(window, cx);
            harness.open = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.render_frame(cx));
        let phases = phases.borrow();
        assert!(phases.len() >= 2);
        // 无辅助功能适配器的测试平台也能捕获根因：同帧开始登记焦点后，子树不得再切换焦点。
        assert!(
            phases.chunks_exact(2).all(|frame| frame[0] == frame[1]),
            "展开内容在 prepaint 中更改了焦点，原生辅助功能会重复登记：{phases:?}"
        );
    }

    struct ShadowPanel {
        app: Entity<AppView>,
        _subscription: Subscription,
    }

    impl Render for ShadowPanel {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().w_80().p_4().child(self.app.update(cx, |app, cx| {
                app.render_image_section(cx).into_any_element()
            }))
        }
    }

    #[gpui_kit::test]
    fn shadow_directions_and_external_values_keep_controls_in_sync(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let slot = slot.clone();
            move |window, cx| {
                let app = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                slot.borrow_mut().replace(app.clone());
                let panel = cx.new(|cx| ShadowPanel {
                    _subscription: cx.observe(&app, |_, _, cx| cx.notify()),
                    app,
                });
                Root::new(panel, window, cx)
            }
        });
        let app = slot.borrow().clone().unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| {
            // 真正点击渲染后的滑轨：若阴影浓度退回纯输入框，这里会找不到目标。
            window
                .within("shadow-density")
                .click("slider-bar-container", cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!((app.backend.params().shadow_density - 2.0).abs() < 0.06)
        });
        app.read_with(cx, |app, cx| {
            assert_eq!(
                app.backend.params().shadow_direction,
                ShadowDirection::Center
            );
            assert_eq!(app.backend.params().shadow_offset, 0.015);
            assert!((app.controls.shadow_offset.value(cx) - 0.015).abs() < 1e-6);
        });
        cx.update(|window, cx| {
            let bounds = window
                .within("shadow-offset")
                .find("slider-bar-container")
                .bounds();
            window.within("shadow-offset").click_at(
                "slider-bar-container",
                gpui_kit::point(bounds.size.width * 0.25, bounds.size.height / 2.0),
                cx,
            );
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!((app.backend.params().shadow_offset + 0.05).abs() < 0.001)
        });
        for (id, expected) in [
            ("shadow-up-left", ShadowDirection::UpLeft),
            ("shadow-up", ShadowDirection::Up),
            ("shadow-up-right", ShadowDirection::UpRight),
            ("shadow-right", ShadowDirection::Right),
            ("shadow-down-right", ShadowDirection::DownRight),
            ("shadow-down", ShadowDirection::Down),
            ("shadow-down-left", ShadowDirection::DownLeft),
            ("shadow-left", ShadowDirection::Left),
            ("shadow-center", ShadowDirection::Center),
        ] {
            cx.update(|window, cx| window.click(id, cx));
            cx.run_until_parked();
            app.read_with(cx, |app, cx| {
                assert_eq!(app.backend.params().shadow_direction, expected);
                assert!((app.backend.params().shadow_offset + 0.05).abs() < 0.001);
                assert!((app.controls.shadow_offset.value(cx) + 0.05).abs() < 0.001);
            });
        }
        // 方向切换不改距离；预设/照片切换同步单一距离和轮廓缩放。
        app.update_in(cx, |app, window, cx| {
            app.backend.params_mut().shadow_offset = 0.065;
            app.backend.params_mut().shadow_scale = 0.96;
            app.sync_controls(window, cx);
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.click("shadow-down-left", cx));
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(
                app.backend.params().shadow_direction,
                ShadowDirection::DownLeft
            );
            assert_eq!(app.backend.params().shadow_offset, 0.065);
            assert!((app.controls.shadow_offset.value(cx) - 0.065).abs() < 1e-6);
            assert!((app.controls.shadow_scale.value(cx) - 0.96).abs() < 1e-6);
        });
        cx.update(|window, cx| {
            let bounds = window
                .within("shadow-scale")
                .find("slider-bar-container")
                .bounds();
            window.within("shadow-scale").click_at(
                "slider-bar-container",
                gpui_kit::point(bounds.size.width * 0.5, bounds.size.height / 2.0),
                cx,
            );
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!((app.backend.params().shadow_scale - 0.85).abs() < 0.002)
        });
    }
}
