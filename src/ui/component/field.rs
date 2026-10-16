//! 参数面板里可复用的输入控件。
//!
//! 面板里的每个数值和颜色都同时提供「拖着调」和「打字调」两条路：滑块负责找感觉，
//! 输入框负责给准数。两者共用同一条写回路径，因此不会出现读数与参数不一致的情况。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _,
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    form::Field,
    h_flex,
    input::{Input, InputEvent, InputState, NumberInput, NumberStep},
    searchable_list::SearchableListItem,
    select::{SelectEvent, SelectState},
    slider::{Slider, SliderEvent, SliderScale, SliderState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, Hsla, Rgba, SharedString, Subscription, Window, div,
};

use super::super::AppView;

/// 给下拉框用的「标签 + 领域值」选项。
///
/// 直接把选项做成字符串的话，选中结果还要再靠文本反解回领域值；这里让下拉框把领域值
/// 本身带回来。
#[derive(Clone)]
pub(in crate::ui::app) struct Choice<T: Clone + PartialEq + 'static> {
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

/// 把「标签 + 领域值」列表变成下拉框选项。
pub(super) fn choices<T: Clone + PartialEq + 'static>(entries: &[(&str, T)]) -> Vec<Choice<T>> {
    entries
        .iter()
        .map(|(label, value)| Choice::new(*label, value.clone()))
        .collect()
}

/// 选中某个值时对应的下拉位置。
pub(in crate::ui::app) fn index_of<T: Clone + PartialEq + 'static>(
    entries: &[(&str, T)],
    value: &T,
) -> Option<IndexPath> {
    entries
        .iter()
        .position(|(_, candidate)| candidate == value)
        .map(IndexPath::new)
}

/// 一个数值参数的两种输入方式：滑块拖动 + 键盘精确输入。
///
/// 界面上的读数用「显示单位」（例如百分比），模型里存的是原始比例，两者之间的换算由
/// `scale` 承担，回调拿到的永远是模型值。
pub(in crate::ui::app) struct NumberField {
    slider: Entity<SliderState>,
    input: Entity<InputState>,
    /// 显示值 = 模型值 × scale。
    scale: f64,
    unit: &'static str,
    decimals: usize,
    min: f64,
    max: f64,
    /// 对数滑块内部使用正数，显示和业务值仍从原下限开始。
    slider_offset: f64,
}

impl NumberField {
    /// 只呈现精确输入，适用于已有方向等快捷操作的参数。
    pub(in crate::ui::app) fn render_input(&self) -> impl IntoElement {
        h_flex()
            .gap_1()
            .child(Input::new(&self.input).id("number-input").small().w_20())
            .child(div().text_sm().child(self.unit))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui::app) fn new(
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        decimals: usize,
        scale: f64,
        unit: &'static str,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let display = value * scale + 0.0;
        let slider = cx.new(|_| {
            SliderState::new()
                .max(max as f32)
                // `SliderState::min` immediately recomputes the thumb with its current max
                // (100 by default), so ranges above 100 must install max first.
                .min(min as f32)
                .step(step as f32)
                .default_value(display as f32)
        });
        let input = cx
            .new(|cx| InputState::new(window, cx).default_value(format_number(display, decimals)));

        Self {
            slider,
            input,
            scale,
            unit,
            decimals,
            min,
            max,
            slider_offset: 0.0,
        }
    }

    /// 在订阅前配置原生对数刻度，把更多拖动空间留给常用的小数值。
    pub(in crate::ui::app) fn logarithmic(
        mut self,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let display = self.value(cx) * self.scale;
        let step = self.slider.read(cx).step_value();
        // 原生对数刻度不接受 0；平移区间保留真正的 0%，而不是抬高用户可设置的下限。
        self.slider_offset = 1.0 - self.min;
        self.input.update(cx, |state, cx| {
            state.set_step(
                Some(NumberStep::Fixed(10_f64.powi(-(self.decimals as i32)))),
                window,
                cx,
            );
            state.set_min(Some(self.min), window, cx);
            state.set_max(Some(self.max), window, cx);
        });
        let offset = self.slider_offset;
        self.slider = cx.new(|_| {
            SliderState::new()
                .max((self.max + offset) as f32)
                .min(1.0)
                .step(step)
                .scale(SliderScale::Logarithmic)
                .default_value((display + offset) as f32)
        });
        self
    }

    /// 模型值。
    pub(super) fn value(&self, cx: &App) -> f64 {
        (f64::from(self.slider.read(cx).value().end()) - self.slider_offset) / self.scale
    }

    /// 把两条输入路径接到同一个写回动作上。
    pub(in crate::ui::app) fn subscribe(
        &self,
        window: &mut Window,
        cx: &mut Context<AppView>,
        apply: impl Fn(&mut AppView, f64) + Clone + 'static,
    ) -> Vec<Subscription> {
        let from_slider = {
            let input = self.input.clone();
            let (scale, decimals, offset) = (self.scale, self.decimals, self.slider_offset);
            let apply = apply.clone();
            cx.subscribe_in(&self.slider, window, move |this, _, event, window, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                let model = (f64::from(value.end()) - offset) / scale;
                apply(this, model);
                write_input(&input, model * scale, decimals, window, cx);
                this.refresh_preview(cx);
            })
        };

        let from_input = {
            let slider = self.slider.clone();
            let (min, max, scale, decimals, offset) = (
                self.min,
                self.max,
                self.scale,
                self.decimals,
                self.slider_offset,
            );
            cx.subscribe_in(
                &self.input,
                window,
                move |this, state, event, window, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let text = state.read(cx).value().to_string();
                    let Some(typed) = parse_number(&text) else {
                        return;
                    };
                    let display = typed.clamp(min, max);
                    apply(this, display / scale);

                    // 抄了范围就把输入框拉回真实值，别让框里的数字和参数对不上。
                    if (display - typed).abs() > f64::EPSILON {
                        write_input(state, display, decimals, window, cx);
                    }
                    if (f64::from(slider.read(cx).value().end()) - (display + offset)).abs() > 1e-9
                    {
                        slider.update(cx, |state, cx| {
                            state.set_value((display + offset) as f32, window, cx)
                        });
                    }
                    this.refresh_preview(cx);
                },
            )
        };

        vec![from_slider, from_input]
    }

    /// 由外部（载入预设）改写数值，滑块与输入框一起跟上。
    pub(in crate::ui::app) fn sync(&self, value: f64, window: &mut Window, cx: &mut App) {
        let display = value * self.scale;
        self.slider.update(cx, |state, cx| {
            state.set_value((display + self.slider_offset) as f32, window, cx)
        });
        write_input(&self.input, display, self.decimals, window, cx);
    }

    pub(in crate::ui::app) fn render(
        &self,
        label: impl Into<SharedString>,
        disabled: bool,
        cx: &App,
    ) -> AnyElement {
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(label_color(cx, disabled))
                            .child(label.into()),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .items_center()
                            .child(if self.slider_offset != 0.0 {
                                NumberInput::new(&self.input)
                                    .small()
                                    .w_32()
                                    .disabled(disabled)
                                    .into_any_element()
                            } else {
                                Input::new(&self.input)
                                    .small()
                                    .w_20()
                                    .disabled(disabled)
                                    .into_any_element()
                            })
                            .when(!self.unit.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(self.unit),
                                )
                            }),
                    ),
            )
            .child(Slider::new(&self.slider).disabled(disabled).w_full())
            .into_any_element()
    }
}

/// 一个颜色参数：取色器 + 十六进制输入框。
///
/// 十六进制输入是并列的第二条路径，不是取色器的替代：取色器崩了或者只是懒得点开时，
/// 直接敲 `#1A2B3C` 一样能改。
#[derive(Clone)]
pub(in crate::ui::app) struct ColorField {
    picker: Entity<ColorPickerState>,
    hex: Entity<InputState>,
}

impl ColorField {
    /// 用于已有行标签的紧凑颜色编辑器。
    pub(in crate::ui::app) fn render_control(&self) -> impl IntoElement {
        let picker = div()
            .id("field-color-picker")
            .child(ColorPicker::new(&self.picker));
        #[cfg(test)]
        let picker = {
            use gpui_kit::test::TestSupportExt as _;
            picker.test_support()
        };
        h_flex().gap_2().items_center().child(picker).child(
            Input::new(&self.hex)
                .id("gradient-color-hex")
                .aria_label("颜色值")
                .small()
                .w_24(),
        )
    }

    pub(in crate::ui::app) fn new(
        rgb: [u8; 3],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb_to_hsla(rgb)));
        let hex = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(hex_string(rgb))
                .placeholder("#RRGGBB")
        });
        Self { picker, hex }
    }

    pub(super) fn is_open(&self, cx: &App) -> bool {
        self.picker.read(cx).is_open()
    }

    pub(super) fn value(&self, cx: &App) -> [u8; 3] {
        self.picker.read(cx).value().map_or([0, 0, 0], hsla_to_rgb)
    }

    pub(in crate::ui::app) fn subscribe(
        &self,
        window: &mut Window,
        cx: &mut Context<AppView>,
        apply: impl Fn(&mut AppView, [u8; 3]) + Clone + 'static,
    ) -> Vec<Subscription> {
        self.subscribe_edit(window, cx, move |this, rgb, _, _| apply(this, rgb))
    }

    pub(in crate::ui::app) fn subscribe_edit(
        &self,
        window: &mut Window,
        cx: &mut Context<AppView>,
        apply: impl Fn(&mut AppView, [u8; 3], &mut Window, &mut Context<AppView>) + Clone + 'static,
    ) -> Vec<Subscription> {
        let from_picker = {
            let hex = self.hex.clone();
            let apply = apply.clone();
            cx.subscribe_in(&self.picker, window, move |this, _, event, window, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                let rgb = hsla_to_rgb(*color);
                apply(this, rgb, window, cx);

                // 输入框里已经是这个颜色就别动它，免得打字打到一半被规范化。
                let typed = hex.read(cx).value().to_string();
                if parse_hex(&typed) != Some(rgb) {
                    write_text(&hex, hex_string(rgb), window, cx);
                }
                this.refresh_preview(cx);
                cx.notify();
            })
        };

        let from_hex = {
            let picker = self.picker.clone();
            cx.subscribe_in(&self.hex, window, move |this, state, event, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = state.read(cx).value().to_string();
                let Some(rgb) = parse_hex(&text) else {
                    return;
                };
                apply(this, rgb, window, cx);
                let color = rgb_to_hsla(rgb);
                if picker.read(cx).value() != Some(color) {
                    picker.update(cx, |state, cx| state.update_color(color, window, cx));
                }
                this.refresh_preview(cx);
                cx.notify();
            })
        };

        // ColorPicker 是 RenderOnce，打开面板、悬浮预览和切换选色模式只通知 picker 状态。
        // 拥有控件的视图也要重绘，才能把这些状态更新到弹层里。
        let picker_observer = cx.observe(&self.picker, |_, _, cx| cx.notify());
        vec![from_picker, from_hex, picker_observer]
    }

    pub(in crate::ui::app) fn sync(&self, rgb: [u8; 3], window: &mut Window, cx: &mut App) {
        let color = rgb_to_hsla(rgb);
        self.picker
            .update(cx, |state, cx| state.set_value(color, window, cx));
        write_text(&self.hex, hex_string(rgb), window, cx);
    }

    pub(in crate::ui::app) fn render(
        &self,
        label: impl Into<SharedString>,
        disabled: bool,
        cx: &App,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(label_color(cx, disabled))
                    .child(label.into()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(ColorPicker::new(&self.picker))
                    .child(Input::new(&self.hex).small().w_24()),
            )
            .into_any_element()
    }
}

/// 一行「标签 + 下拉框」的字段。
pub(in crate::ui::app) fn field(
    label: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Field {
    Field::new().w_full().flex_initial().child(
        v_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(label.into()),
            )
            .child(control),
    )
}

/// 不依附于单个控件的说明，复用表单字段的描述样式。
pub(in crate::ui::app) fn description(text: impl Into<SharedString>) -> AnyElement {
    Field::new()
        .flex_initial()
        .gap_0()
        .description(text.into())
        .into_any_element()
}

/// 摘要、状态和标签等次要文字。
pub(super) fn muted_text(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// 提示当前值有问题时用的说明文字。
pub(super) fn warning(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .gap_1()
        .items_center()
        .text_xs()
        .text_color(cx.theme().danger)
        .child(
            gpui_kit::component::Icon::new(gpui_kit::component::IconName::TriangleAlert).xsmall(),
        )
        .child(text.into())
        .into_any_element()
}

fn label_color(cx: &App, disabled: bool) -> Hsla {
    if disabled {
        cx.theme().muted_foreground
    } else {
        cx.theme().foreground
    }
}

fn format_number(display: f64, decimals: usize) -> String {
    format!("{display:.decimals$}")
}

fn parse_number(text: &str) -> Option<f64> {
    let cleaned = text.trim().trim_end_matches('%').trim();
    if cleaned.is_empty() {
        return None;
    }
    let value: f64 = cleaned.parse().ok()?;
    value.is_finite().then_some(value)
}

/// 把显示值写进输入框。`set_value` 不会发出 `Change`，所以不会形成回环。
fn write_input(
    input: &Entity<InputState>,
    display: f64,
    decimals: usize,
    window: &mut Window,
    cx: &mut App,
) {
    write_text(input, format_number(display, decimals), window, cx);
}

fn write_text(input: &Entity<InputState>, text: String, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() == text {
        return;
    }
    input.update(cx, |state, cx| state.set_value(text, window, cx));
}

pub(super) fn hex_string(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// 解析 `#RRGGBB` / `RRGGBB` / 三位缩写。
pub(super) fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let cleaned = text.trim().trim_start_matches('#').trim();
    let expanded = match cleaned.len() {
        3 => cleaned.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
        6 => cleaned.to_string(),
        _ => return None,
    };
    let value = u32::from_str_radix(&expanded, 16).ok()?;
    let [_, r, g, b] = value.to_be_bytes();
    Some([r, g, b])
}

pub(super) fn hsla_to_rgb(color: Hsla) -> [u8; 3] {
    let rgba: Rgba = color.into();
    [
        (rgba.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub(in crate::ui::app) fn rgb_to_hsla(rgb: [u8; 3]) -> Hsla {
    Rgba {
        r: rgb[0] as f32 / 255.0,
        g: rgb[1] as f32 / 255.0,
        b: rgb[2] as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

/// 选中状态由外部维护的下拉框：构建时只给出选项与当前选中项。
pub(super) fn select_state<T>(
    items: Vec<Choice<T>>,
    selected: Option<IndexPath>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Entity<SelectState<Vec<Choice<T>>>>
where
    T: Clone + PartialEq + 'static,
{
    // 面板里的下拉都可能很长（字体尤其），一律开启搜索。
    cx.new(|cx| SelectState::new(items, selected, window, cx).searchable(true))
}

/// 订阅下拉框的确认事件。
pub(super) fn on_select<T>(
    state: &Entity<SelectState<Vec<Choice<T>>>>,
    window: &mut Window,
    cx: &mut Context<AppView>,
    apply: impl Fn(&mut AppView, T, &App) + 'static,
) -> Subscription
where
    T: Clone + PartialEq + 'static,
{
    cx.subscribe_in(state, window, move |this, _, event, _, cx| {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        apply(this, value.clone(), cx);
        this.refresh_preview(cx);
        cx.notify();
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Render, TestAppContext};
    struct BorderControl {
        app: Entity<AppView>,
    }
    impl Render for BorderControl {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w_96()
                .p_4()
                .child(
                    self.app
                        .read(cx)
                        .controls
                        .border_top
                        .render("上边框", false, cx),
                )
        }
    }
    #[gpui_kit::test]
    fn border_slider_keeps_fine_steps_and_full_range(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
            let view = cx.new(|_| BorderControl { app });
            Root::new(view, window, cx)
        });
        let view = view.read_with(cx, |root, _| {
            root.view()
                .clone()
                .downcast::<BorderControl>()
                .ok()
                .unwrap()
        });
        let app = view.read_with(cx, |view, _| view.app.clone());
        let input = app.read_with(cx, |app, _| app.controls.border_top.input.clone());
        cx.run_until_parked();
        for value in [0., 40., 100., 200.] {
            cx.update(|window, cx| {
                window.click(("input", input.entity_id()), cx);
                window.press("cmd-a", cx);
                window.input(&value.to_string(), cx);
            });
            cx.run_until_parked();
            app.read_with(cx, |app, cx| {
                assert!((app.params.border_ratio.0 * 100. - value).abs() < 0.0001);
                assert!((app.controls.border_top.value(cx) * 100. - value).abs() < 0.0001);
                if value == 40. {
                    assert!(app.controls.border_top.slider.read(cx).percentage().end > 0.65);
                }
            });
            cx.update(|window, cx| window.click("increment", cx));
            cx.run_until_parked();
            app.read_with(cx, |app, _| {
                assert!(
                    (app.params.border_ratio.0 * 100. - (value + 0.1_f64).min(200.)).abs() < 0.0001
                )
            });
        }
    }
}
