//! 参数面板里的 EXIF 文字水印编辑区。
//!
//! 一行文字由若干控件组成（模板、字号、行距、字体、颜色、粗体斜体、对齐）。这些控件实体
//! 本身就是这一行的真值来源：`Text` 在需要时由它们拼出来，所以不存在「控件显示的值」和
//! 「模型里的值」两份状态互相漂移的问题。
//!
//! 每行默认折叠：一屏里往往只有一两行需要调，全展开会让面板长到看不见别的参数。

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, Sizable as _,
    accordion::{Accordion, AccordionItem},
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, IntoElement, SharedString, Subscription, Window, div};

use crate::photo::ExifInfo;
use crate::process::text::{
    TIME_FORMAT_EXAMPLES, TextAlign, TextParams, render_exif_template, time_format_is_valid,
};

use super::AppView;
use super::field::{
    Choice, ColorField, NumberField, choices, field, hint, index_of, on_select, select_state,
    warning,
};

/// 模板里可以使用的字段。
///
/// `{Logo}` 在渲染时会被替换成对应相机的品牌标志图片，其余字段来自 EXIF。缺失的字段会
/// 被整段丢弃，不会留下 `{xxx}` 字面量。
const TEMPLATE_FIELDS: &str = "{拍摄日期} {品牌} {型号} {镜头型号} {快门} {光圈} {ISO} {曝光补偿} {实际焦距} {等效焦距} {Logo} {GPS} {省} {市} {区}";

/// 对齐方式的选项。
pub(super) const TEXT_ALIGNS: &[(&str, TextAlign)] = &[
    ("左对齐", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("右对齐", TextAlign::Right),
];

/// 每行文字水印的字体下拉。
type FontSelect = SelectState<Vec<SharedString>>;
type AlignSelect = SelectState<Vec<Choice<TextAlign>>>;

/// 一行文字水印的完整状态。
pub(super) struct TextLine {
    /// 稳定的行身份。行的增删不会让别的行的控件状态串位。
    pub id: u64,
    pub template: Entity<InputState>,
    pub size: NumberField,
    pub line_spacing: NumberField,
    pub font: Entity<FontSelect>,
    pub align: Entity<AlignSelect>,
    pub color: ColorField,
    pub auto_color: bool,
    pub bold: bool,
    pub italic: bool,
}

impl TextLine {
    pub(super) fn new(
        id: u64,
        template: &str,
        params: &TextParams,
        fonts: &[SharedString],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> (Self, Vec<Subscription>) {
        let mut subscriptions = Vec::new();

        let template_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(template.to_owned())
                .placeholder("例如 {Logo} {型号}")
        });
        let size = NumberField::new(params.size, 1.0, 12.0, 0.1, 1, 100.0, "%", window, cx);
        let line_spacing =
            NumberField::new(params.line_spacing, 1.0, 2.5, 0.05, 2, 1.0, "", window, cx);

        // 配置里记着的字体不一定还装在这台机器上；把它放进列表里，至少不会在选择框里
        // 凭空消失，用户能看出原本用的是什么。
        let configured = SharedString::from(params.font.clone());
        let mut font_items: Vec<SharedString> = Vec::with_capacity(fonts.len() + 1);
        if !fonts.iter().any(|font| font == &configured) {
            font_items.push(configured.clone());
        }
        font_items.extend(fonts.iter().cloned());
        let font_index = font_items
            .iter()
            .position(|font| font == &configured)
            .unwrap_or(0);
        let font =
            cx.new(|cx| SelectState::new(font_items, Some(IndexPath::new(font_index)), window, cx));

        let align = select_state(
            choices(TEXT_ALIGNS),
            index_of(TEXT_ALIGNS, &params.align),
            window,
            cx,
        );
        let color = ColorField::new(params.color.unwrap_or([255, 255, 255]), window, cx);

        // 模板变化时不只是重算预览：这一行的「解析结果」提示也要跟着刷新。
        subscriptions.push(
            cx.subscribe_in(&template_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );
        // 字号与行距直接存在控件里，没有第二个地方要写，所以只需要重算预览。
        subscriptions.extend(size.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(line_spacing.subscribe(window, cx, |_, _| {}));
        subscriptions.push(cx.subscribe_in(&font, window, |this, _, event, _, cx| {
            if let SelectEvent::Confirm(Some(_)) = event {
                this.refresh_preview(cx);
                cx.notify();
            }
        }));
        subscriptions.push(on_select(&align, window, cx, |_, _, _| {}));
        subscriptions.extend(color.subscribe(window, cx, |_, _| {}));

        (
            Self {
                id,
                template: template_input,
                size,
                line_spacing,
                font,
                align,
                color,
                auto_color: params.color.is_none(),
                bold: params.bold,
                italic: params.italic,
            },
            subscriptions,
        )
    }

    /// 这一行的字体。
    pub(super) fn font_name(&self, cx: &App) -> String {
        self.font
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default()
            .to_string()
    }

    /// 这一行的对齐方式。
    pub(super) fn alignment(&self, cx: &App) -> TextAlign {
        self.align
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default()
    }
}

/// 常用时间格式的选项：显示一个真实例子，值就是模板本身。
pub(super) fn time_format_choices() -> Vec<Choice<String>> {
    TIME_FORMAT_EXAMPLES
        .iter()
        .map(|(example, template)| Choice::new(*example, (*template).to_string()))
        .collect()
}

/// 文字位置说明。
pub(super) fn position_hint(position: crate::Position) -> &'static str {
    match position {
        crate::Position::Center => "居中：文字压在照片正中。",
        crate::Position::Up => "靠上：文字排在照片上方的边框里。",
        crate::Position::Bottom => "靠下：文字排在照片下方的边框里。",
        crate::Position::Left => "靠左：文字旋转后排进左边框。",
        crate::Position::Right => "靠右：文字旋转后排进右边框。",
    }
}

impl AppView {
    pub(super) fn render_text_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let exif = self
            .selected_photo()
            .and_then(|photo| photo.exif().cloned());
        let time_format = self.time_format.clone();

        // 组件默认铺满父级高度，在自动高度的分组里显式交回去。
        let mut lines = Accordion::new("text-lines")
            .multiple(true)
            .bordered(true)
            .h_auto();
        for (ix, line) in self.text_lines.iter().enumerate() {
            lines = lines
                .item(|item| self.text_line_item(item, ix, line, exif.as_ref(), &time_format, cx));
        }

        GroupBox::new()
            .id("text-section")
            .title("EXIF 文字水印")
            .child(field(
                "文字位置",
                Select::new(&self.controls.text_position).w_full(),
                cx,
            ))
            .child(hint(position_hint(self.text_position), cx))
            .child(field(
                "时间格式",
                Input::new(&self.controls.time_format),
                cx,
            ))
            .child(field(
                "常用格式",
                Select::new(&self.controls.time_format_example).w_full(),
                cx,
            ))
            .child(if time_format_is_valid(&time_format) {
                hint(
                    "占位符遵循 strftime：%Y 年、%m 月、%d 日、%H:%M:%S 时间、%A 星期。",
                    cx,
                )
            } else {
                warning("时间格式无效，将使用默认的 %Y/%m/%d。", cx)
            })
            .child(hint(format!("模板可用字段：{TEMPLATE_FIELDS}"), cx))
            .child(
                lines.on_toggle_click(cx.listener(|this, open: &[usize], _, cx| {
                    this.expanded_text_lines = open
                        .iter()
                        .filter_map(|ix| this.text_lines.get(*ix).map(|line| line.id))
                        .collect();
                    cx.notify();
                })),
            )
            .child(
                Button::new("text-add-line")
                    .icon(IconName::Plus)
                    .label("添加一行")
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.add_text_line(window, cx))),
            )
    }

    /// 组装单个文字行的折叠项。
    fn text_line_item(
        &self,
        item: AccordionItem,
        ix: usize,
        line: &TextLine,
        exif: Option<&ExifInfo>,
        time_format: &str,
        cx: &Context<Self>,
    ) -> AccordionItem {
        let id = line.id;
        let template = line.template.read(cx).value().to_string();
        let summary = if template.trim().is_empty() {
            "（空模板）".to_string()
        } else {
            template.clone()
        };

        let title = h_flex()
            .w_full()
            .min_w_0()
            .justify_between()
            .gap_2()
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(format!("第 {} 行", ix + 1)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(summary),
                    ),
            )
            .child(
                Button::new(("text-remove-line", id))
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip("删除这一行")
                    .accessibility_label(format!("删除第 {} 行", ix + 1))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_text_line(id, cx))),
            );

        // `{Logo}` 由排版阶段替换成品牌标志图片，这里先标出来，免得用户以为字段没生效。
        let resolved = match exif {
            None => warning("选中的照片没有 EXIF 信息，这一行不会渲染。", cx),
            Some(exif) => {
                let preview = template.replace("{Logo}", "[品牌 Logo]");
                let resolved = render_exif_template(&preview, exif, time_format);
                if resolved.is_empty() {
                    warning("当前模板解析结果为空，这一行不会渲染。", cx)
                } else {
                    hint(format!("解析结果：{resolved}"), cx)
                }
            }
        };

        item.open(self.expanded_text_lines.contains(&id))
            .title(title)
            .child(Input::new(&line.template))
            .child(resolved)
            .child(line.size.render("字号", false, cx))
            .child(line.line_spacing.render("行距", false, cx))
            .child(field("字体", Select::new(&line.font).w_full(), cx))
            .child(field("对齐", Select::new(&line.align).w_full(), cx))
            .child(line.color.render("文字颜色", line.auto_color, cx))
            .child(
                Switch::new(("text-auto-color", id))
                    .checked(line.auto_color)
                    .label("自动颜色（按背景取黑或白）")
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
