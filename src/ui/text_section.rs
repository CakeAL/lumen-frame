//! 参数面板里的 EXIF 文字水印编辑区。
//!
//! 一行文字由若干控件组成（模板、字号、行距、颜色、粗体斜体、对齐）。这些控件实体本身
//! 就是这一行的真值来源：`Text` 在需要时由它们拼出来，所以不存在「控件显示的值」和
//! 「模型里的值」两份状态互相漂移的问题。

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, Sizable as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    slider::{SliderEvent, SliderState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, SharedString, Subscription, Window, div};

use crate::process::text::{TextAlign, TextParams, render_exif_template};

use super::AppView;
use super::inspector::{
    Choice, TEXT_ALIGNS, choices, field, hint, percent, rgb_to_hsla, slider_row, slider_value,
};

/// 模板里可以使用的字段。
///
/// `{Logo}` 在渲染时会被替换成对应相机的品牌标志图片，其余字段来自 EXIF。缺失的字段
/// 会被整段丢弃，不会留下 `{xxx}` 字面量。
const TEMPLATE_FIELDS: &str = "{拍摄日期} {品牌} {型号} {镜头型号} {快门} {光圈} {ISO} {曝光补偿} {实际焦距} {等效焦距} {Logo}";

/// 一行文字水印的完整状态。
pub(super) struct TextLine {
    /// 稳定的行身份。行的增删不会让别的行的控件状态串位。
    pub id: u64,
    pub template: Entity<InputState>,
    pub size: Entity<SliderState>,
    pub line_spacing: Entity<SliderState>,
    pub align: Entity<SelectState<Vec<Choice<TextAlign>>>>,
    pub color: Entity<ColorPickerState>,
    /// 字体暂时不在界面上暴露，但必须原样带进 [`TextParams`]。
    pub font: SharedString,
    pub auto_color: bool,
    pub bold: bool,
    pub italic: bool,
}

impl TextLine {
    pub(super) fn new(
        id: u64,
        template: &str,
        params: &TextParams,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> (Self, Vec<Subscription>) {
        let template_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(template.to_owned())
                .placeholder("例如 {Logo} {型号}")
        });
        let size = cx.new(|_| {
            SliderState::new()
                .min(0.01)
                .max(0.12)
                .step(0.001)
                .default_value(params.size as f32)
        });
        let line_spacing = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(2.5)
                .step(0.05)
                .default_value(params.line_spacing as f32)
        });
        let align = cx.new(|cx| {
            let items = choices(TEXT_ALIGNS);
            let selected = TEXT_ALIGNS
                .iter()
                .position(|(_, value)| *value == params.align)
                .map(IndexPath::new);
            SelectState::new(items, selected, window, cx)
        });
        let color = cx.new(|cx| {
            ColorPickerState::new(window, cx).default_value(
                params
                    .color
                    .map_or_else(|| rgb_to_hsla([255, 255, 255]), rgb_to_hsla),
            )
        });

        let mut subscriptions = Vec::new();

        // 模板变化时不只是重算预览：每一行的「解析结果」提示也要跟着刷新。
        subscriptions.push(
            cx.subscribe_in(&template_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.subscribe(&size, |this, _, event, cx| {
            if matches!(event, SliderEvent::Change(_)) {
                this.refresh_preview(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&line_spacing, |this, _, event, cx| {
            if matches!(event, SliderEvent::Change(_)) {
                this.refresh_preview(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&align, |this, _, event, cx| {
            let SelectEvent::Confirm(_) = event;
            this.refresh_preview(cx);
            cx.notify();
        }));
        subscriptions.push(cx.subscribe(&color, |this, _, event, cx| {
            if matches!(event, ColorPickerEvent::Change(_)) {
                this.refresh_preview(cx);
            }
        }));

        (
            Self {
                id,
                template: template_input,
                size,
                line_spacing,
                align,
                color,
                font: SharedString::from(params.font.clone()),
                auto_color: params.color.is_none(),
                bold: params.bold,
                italic: params.italic,
            },
            subscriptions,
        )
    }
}

impl AppView {
    pub(super) fn render_text_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let exif = self.selected_photo().and_then(|photo| photo.exif.clone());
        let time_format = self.time_format.clone();

        GroupBox::new()
            .id("text-section")
            .title("EXIF 文字水印")
            .child(field(
                "文字位置",
                Select::new(&self.controls.text_position).w_full(),
                cx,
            ))
            .child(field(
                "时间格式",
                Input::new(&self.controls.time_format),
                cx,
            ))
            .child(hint(format!("可用字段：{TEMPLATE_FIELDS}"), cx))
            .children(self.text_lines.iter().enumerate().map(|(ix, line)| {
                self.render_text_line(ix, line, exif.as_ref(), time_format.as_str(), cx)
            }))
            .child(
                Button::new("text-add-line")
                    .icon(IconName::Plus)
                    .label("添加一行")
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.add_text_line(window, cx))),
            )
    }

    fn render_text_line(
        &self,
        ix: usize,
        line: &super::text_section::TextLine,
        exif: Option<&crate::photo::ExifInfo>,
        time_format: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let id = line.id;
        let template = line.template.read(cx).value().to_string();
        let resolved = match exif {
            Some(exif) => {
                // `{Logo}` 由排版阶段替换成品牌标志图片，模板解析这里先标出来，
                // 免得用户以为字段没生效。
                let preview = template.replace("{Logo}", "[品牌 Logo]");
                render_exif_template(&preview, exif, time_format)
            }
            None => String::new(),
        };

        v_flex()
            .w_full()
            .gap_3()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("第 {} 行", ix + 1)),
                    )
                    .child(
                        Button::new(("text-remove-line", id))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("删除这一行")
                            .accessibility_label(format!("删除第 {} 行", ix + 1))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_text_line(id, cx)),
                            ),
                    ),
            )
            .child(Input::new(&line.template))
            .child(if exif.is_none() {
                hint("选中的照片没有 EXIF 信息，这一行不会渲染。", cx).into_any_element()
            } else if resolved.is_empty() {
                hint("当前模板解析结果为空，这一行不会渲染。", cx).into_any_element()
            } else {
                hint(format!("解析结果：{resolved}"), cx).into_any_element()
            })
            .child(slider_row(
                "字号",
                percent(slider_value(&line.size, cx)),
                &line.size,
                false,
                cx,
            ))
            .child(slider_row(
                "行距",
                format!("{:.2}", slider_value(&line.line_spacing, cx)),
                &line.line_spacing,
                false,
                cx,
            ))
            .child(field("对齐", Select::new(&line.align).w_full(), cx))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(if line.auto_color {
                                cx.theme().muted_foreground
                            } else {
                                cx.theme().foreground
                            })
                            .child("文字颜色"),
                    )
                    .child(ColorPicker::new(&line.color)),
            )
            .child(
                Switch::new(("text-auto-color", id))
                    .checked(line.auto_color)
                    .label("自动颜色")
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.set_text_line_auto_color(id, *checked, cx)
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(
                        Switch::new(("text-bold", id))
                            .checked(line.bold)
                            .label("加粗")
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.set_text_line_style(id, Some(*checked), None, cx)
                            })),
                    )
                    .child(
                        Switch::new(("text-italic", id))
                            .checked(line.italic)
                            .label("斜体")
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.set_text_line_style(id, None, Some(*checked), cx)
                            })),
                    ),
            )
    }
}
