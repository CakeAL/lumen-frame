//! EXIF 文字组列表与文字组编辑模态窗。
//!
//! [`AppView`] 持有全部文字组及其控件状态；右侧参数面板只展示文字组摘要。点击“编辑…”
//! 后使用 GPUI Kit 的原生 `Dialog` 在窗口中央编辑组级参数和组内文字行，因此焦点陷阱、
//! Esc/遮罩关闭和焦点恢复都由框架负责。

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, Sizable as _, WindowExt as _,
    accordion::{Accordion, AccordionItem},
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogFooter},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, SharedString, Subscription, Window, div,
};

use crate::Position;
use crate::photo::ExifInfo;
use crate::process::text::{
    TIME_FORMAT_EXAMPLES, Text, TextAlign, TextDirection, TextGroup, TextParams,
    render_exif_template, time_format_is_valid,
};

use super::AppView;
use super::field::{
    Choice, ColorField, NumberField, choices, field, hint, index_of, on_select, select_state,
    warning,
};
use super::inspector::POSITIONS;

const TEMPLATE_FIELDS: &str = "{拍摄日期} {品牌} {型号} {镜头型号} {快门} {光圈} {ISO} {曝光补偿} {实际焦距} {等效焦距} {Logo} {GPS} {省} {市} {区}";

pub(super) const TEXT_ALIGNS: &[(&str, TextAlign)] = &[
    ("左对齐", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("右对齐", TextAlign::Right),
];

const GROUP_ALIGNS: &[(&str, TextAlign)] = &[
    ("左侧", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("右侧", TextAlign::Right),
];

const TEXT_DIRECTIONS: &[(&str, TextDirection)] = &[
    ("横排", TextDirection::Horizontal),
    ("竖排（顺时针旋转 90°）", TextDirection::Vertical),
];

type FontSelect = SelectState<Vec<SharedString>>;
type AlignSelect = SelectState<Vec<Choice<TextAlign>>>;
type PositionSelect = SelectState<Vec<Choice<Position>>>;
type DirectionSelect = SelectState<Vec<Choice<TextDirection>>>;
type TimeFormatSelect = SelectState<Vec<Choice<String>>>;

struct TextGroupModal {
    group_id: u64,
    app: Entity<AppView>,
    _subscription: Subscription,
}

impl TextGroupModal {
    fn new(group_id: u64, app: Entity<AppView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            group_id,
            app,
            _subscription: subscription,
        }
    }
}

impl Render for TextGroupModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app
            .read(cx)
            .render_text_group_dialog(self.group_id, self.app.clone(), cx)
    }
}

pub(super) struct TextLine {
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
    _subscriptions: Vec<Subscription>,
}

impl TextLine {
    fn new(
        id: u64,
        template: &str,
        params: &TextParams,
        fonts: &[SharedString],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let template_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(template.to_owned())
                .placeholder("例如 {Logo} {型号}")
        });
        let size = NumberField::new(params.size, 1.0, 12.0, 0.1, 1, 100.0, "%", window, cx);
        let line_spacing =
            NumberField::new(params.line_spacing, 1.0, 2.5, 0.05, 2, 1.0, "", window, cx);

        let configured = SharedString::from(params.font.clone());
        let mut font_items = Vec::with_capacity(fonts.len() + 1);
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

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe_in(&template_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );
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
            _subscriptions: subscriptions,
        }
    }

    fn to_params(&self, cx: &App) -> TextParams {
        TextParams {
            font: self
                .font
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default()
                .to_string(),
            size: self.size.value(cx),
            line_spacing: self.line_spacing.value(cx),
            color: if self.auto_color {
                None
            } else {
                Some(self.color.value(cx))
            },
            italic: self.italic,
            bold: self.bold,
            align: self
                .align
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
        }
    }
}

pub(super) struct TextGroupEditor {
    pub id: u64,
    pub position: Entity<PositionSelect>,
    pub align: Entity<AlignSelect>,
    pub direction: Entity<DirectionSelect>,
    pub time_format: Entity<InputState>,
    pub time_format_example: Entity<TimeFormatSelect>,
    pub lines: Vec<TextLine>,
    pub expanded_line_ids: Vec<u64>,
    _subscriptions: Vec<Subscription>,
}

impl TextGroupEditor {
    pub(super) fn new(
        id: u64,
        group: &TextGroup,
        next_line_id: &mut u64,
        fonts: &[SharedString],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let position = select_state(
            choices(POSITIONS),
            index_of(POSITIONS, &group.position),
            window,
            cx,
        );
        let align = select_state(
            choices(GROUP_ALIGNS),
            index_of(GROUP_ALIGNS, &group.align),
            window,
            cx,
        );
        let direction = select_state(
            choices(TEXT_DIRECTIONS),
            index_of(TEXT_DIRECTIONS, &group.direction),
            window,
            cx,
        );
        let time_format = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(group.time_format.clone())
                .placeholder("%Y/%m/%d")
        });
        let time_format_example = select_state(time_format_choices(), None, window, cx);

        let mut subscriptions = vec![
            on_select(&position, window, cx, |_, _, _| {}),
            on_select(&align, window, cx, |_, _, _| {}),
            on_select(&direction, window, cx, |_, _, _| {}),
        ];
        subscriptions.push(
            cx.subscribe_in(&time_format, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(&time_format_example, window, {
            let input = time_format.clone();
            move |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(template)) = event else {
                    return;
                };
                input.update(cx, |state, cx| {
                    state.set_value(template.clone(), window, cx)
                });
                this.refresh_preview(cx);
                cx.notify();
            }
        }));

        let lines = group
            .text
            .template
            .iter()
            .zip(group.text.text_params.iter())
            .map(|(template, params)| {
                let line_id = *next_line_id;
                *next_line_id += 1;
                TextLine::new(line_id, template, params, fonts, window, cx)
            })
            .collect();

        Self {
            id,
            position,
            align,
            direction,
            time_format,
            time_format_example,
            lines,
            expanded_line_ids: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn to_group(&self, cx: &App) -> TextGroup {
        TextGroup {
            text: Text {
                template: self
                    .lines
                    .iter()
                    .map(|line| line.template.read(cx).value().to_string())
                    .collect(),
                text_params: self.lines.iter().map(|line| line.to_params(cx)).collect(),
            },
            position: self
                .position
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or(Position::Bottom),
            direction: self
                .direction
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            align: self
                .align
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            time_format: self.time_format.read(cx).value().to_string(),
        }
    }
}

fn time_format_choices() -> Vec<Choice<String>> {
    TIME_FORMAT_EXAMPLES
        .iter()
        .map(|(example, template)| Choice::new(*example, (*template).to_string()))
        .collect()
}

fn position_label(position: Position) -> &'static str {
    match position {
        Position::Center => "居中",
        Position::Up => "靠上",
        Position::Bottom => "靠下",
        Position::Left => "靠左",
        Position::Right => "靠右",
    }
}

fn align_label(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "左侧",
        TextAlign::Center => "居中",
        TextAlign::Right => "右侧",
    }
}

fn direction_label(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::Horizontal => "横排",
        TextDirection::Vertical => "竖排",
    }
}

fn position_hint(position: Position) -> &'static str {
    match position {
        Position::Center => "文字组覆盖在照片中间。",
        Position::Up => "文字组排在照片上方的边框里。",
        Position::Bottom => "文字组排在照片下方的边框里。",
        Position::Left => "文字组排在照片左侧的边框里。",
        Position::Right => "文字组排在照片右侧的边框里。",
    }
}

impl AppView {
    pub(super) fn render_text_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        GroupBox::new()
            .id("text-section")
            .title("EXIF 文字水印")
            .child(hint("每个文字组可独立设置位置、方向和多行文字。", cx))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .when(self.text_groups.is_empty(), |this| {
                        this.child(hint("还没有文字组。", cx))
                    })
                    .children(self.text_groups.iter().enumerate().map(|(ix, group)| {
                        self.render_text_group_row(ix, group, view.clone(), cx)
                    })),
            )
            .child(
                Button::new("text-group-add")
                    .icon(IconName::Plus)
                    .label("添加文字组…")
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.add_text_group(window, cx))),
            )
    }

    fn render_text_group_row(
        &self,
        ix: usize,
        group: &TextGroupEditor,
        view: Entity<AppView>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = group.id;
        let value = group.to_group(cx);
        let summary = format!(
            "{} · {} · {} · {} 行",
            position_label(value.position),
            align_label(value.align),
            direction_label(value.direction),
            value.text.template.len()
        );
        let edit_view = view.clone();

        v_flex()
            .id(("text-group-row", id))
            .w_full()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(format!("文字组 {}", ix + 1)),
                    )
                    .child(
                        Button::new(("text-group-remove", id))
                            .icon(IconName::Delete)
                            .ghost()
                            .xsmall()
                            .tooltip("删除这个文字组")
                            .accessibility_label(format!("删除文字组 {}", ix + 1))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_text_group(id, cx)),
                            ),
                    ),
            )
            .child(hint(summary, cx))
            .child(
                Button::new(("text-group-edit", id))
                    .label("编辑…")
                    .small()
                    .w_full()
                    .on_click(move |_, window, cx| {
                        edit_view
                            .update(cx, |this, cx| this.open_text_group_editor(id, window, cx));
                    }),
            )
            .into_any_element()
    }

    pub(super) fn open_text_group_editor(
        &mut self,
        group_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.text_groups.iter().any(|group| group.id == group_id) {
            return;
        }
        let ix = self
            .text_groups
            .iter()
            .position(|group| group.id == group_id)
            .expect("已确认文字组存在");
        let view = cx.entity();
        let modal = cx.new(|cx| TextGroupModal::new(group_id, view, cx));
        window.open_dialog(cx, move |dialog, window, _| {
            let viewport = window.viewport_size();

            dialog
                .title(format!("编辑文字组 {}", ix + 1))
                .w(viewport.width * 0.7)
                .h(viewport.height * 0.8)
                .child(modal.clone())
                .footer(
                    DialogFooter::new().child(
                        DialogAction::new().child(
                            Button::new("text-group-done")
                                .label("完成")
                                .primary()
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                    ),
                )
        });
    }

    fn render_text_group_dialog(
        &self,
        group_id: u64,
        view: Entity<AppView>,
        cx: &App,
    ) -> AnyElement {
        let Some(group) = self.text_groups.iter().find(|group| group.id == group_id) else {
            return div().into_any_element();
        };
        let value = group.to_group(cx);
        let exif = self
            .selected_photo()
            .and_then(|photo| photo.exif().cloned());
        let mut lines = Accordion::new(("text-lines", group_id))
            .multiple(true)
            .bordered(true)
            .h_auto();
        for (ix, line) in group.lines.iter().enumerate() {
            lines = lines.item(|item| {
                self.text_line_item(
                    item,
                    group_id,
                    ix,
                    line,
                    exif.as_ref(),
                    &value.time_format,
                    view.clone(),
                    cx,
                )
            });
        }
        let toggle_view = view.clone();
        let add_view = view.clone();

        v_flex()
            .id(("text-group-editor", group_id))
            .w_full()
            .gap_6()
            .child(
                GroupBox::new()
                    .title("文字组设置")
                    .child(field("位置", Select::new(&group.position).w_full(), cx))
                    .child(hint(position_hint(value.position), cx))
                    .child(field("组对齐", Select::new(&group.align).w_full(), cx))
                    .child(hint(
                        "组对齐只决定文字组沿图片边的位置，不影响每行文字的对齐。",
                        cx,
                    ))
                    .child(field("方向", Select::new(&group.direction).w_full(), cx))
                    .child(field("时间格式", Input::new(&group.time_format), cx))
                    .child(field(
                        "常用格式",
                        Select::new(&group.time_format_example).w_full(),
                        cx,
                    ))
                    .child(if time_format_is_valid(&value.time_format) {
                        hint("占位符遵循 strftime，例如 %Y/%m/%d 和 %H:%M:%S。", cx)
                    } else {
                        warning("时间格式无效，将使用默认的 %Y/%m/%d。", cx)
                    }),
            )
            .child(
                GroupBox::new()
                    .title("文字行")
                    .child(hint(format!("模板可用字段：{TEMPLATE_FIELDS}"), cx))
                    .when(group.lines.is_empty(), |this| {
                        this.child(hint("这个文字组还没有文字行。", cx))
                    })
                    .when(!group.lines.is_empty(), |this| {
                        this.child(lines.on_toggle_click(move |open: &[usize], _, cx| {
                            toggle_view.update(cx, |this, cx| {
                                this.set_expanded_text_lines(group_id, open, cx)
                            });
                        }))
                    })
                    .child(
                        Button::new(("text-add-line", group_id))
                            .icon(IconName::Plus)
                            .label("添加一行")
                            .w_full()
                            .on_click(move |_, window, cx| {
                                add_view.update(cx, |this, cx| {
                                    this.add_text_line(group_id, window, cx)
                                });
                            }),
                    ),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn text_line_item(
        &self,
        item: AccordionItem,
        group_id: u64,
        ix: usize,
        line: &TextLine,
        exif: Option<&ExifInfo>,
        time_format: &str,
        view: Entity<AppView>,
        cx: &App,
    ) -> AccordionItem {
        let id = line.id;
        let template = line.template.read(cx).value().to_string();
        let summary = if template.trim().is_empty() {
            "（空模板）".to_string()
        } else {
            template.clone()
        };
        let remove_view = view.clone();
        let auto_color_view = view.clone();
        let bold_view = view.clone();
        let italic_view = view;
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
                    .on_click(move |_, _, cx| {
                        remove_view.update(cx, |this, cx| this.remove_text_line(id, cx));
                    }),
            );
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
        let open = self
            .text_groups
            .iter()
            .find(|group| group.id == group_id)
            .is_some_and(|group| group.expanded_line_ids.contains(&id));

        item.open(open)
            .title(title)
            .child(Input::new(&line.template))
            .child(resolved)
            .child(line.size.render("字号", false, cx))
            .child(line.line_spacing.render("行距", false, cx))
            .child(field("字体", Select::new(&line.font).w_full(), cx))
            .child(field("行对齐", Select::new(&line.align).w_full(), cx))
            .child(line.color.render("文字颜色", line.auto_color, cx))
            .child(
                Switch::new(("text-auto-color", id))
                    .checked(line.auto_color)
                    .label("自动颜色（按背景取黑或白）")
                    .on_click(move |checked: &bool, _, cx| {
                        auto_color_view.update(cx, |this, cx| {
                            this.set_text_line_auto_color(id, *checked, cx)
                        });
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(
                        Switch::new(("text-bold", id))
                            .checked(line.bold)
                            .label("加粗")
                            .on_click(move |checked: &bool, _, cx| {
                                bold_view.update(cx, |this, cx| {
                                    this.set_text_line_style(id, Some(*checked), None, cx)
                                });
                            }),
                    )
                    .child(
                        Switch::new(("text-italic", id))
                            .checked(line.italic)
                            .label("斜体")
                            .on_click(move |checked: &bool, _, cx| {
                                italic_view.update(cx, |this, cx| {
                                    this.set_text_line_style(id, None, Some(*checked), cx)
                                });
                            }),
                    ),
            )
    }

    pub(super) fn add_text_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_text_group_id;
        self.next_text_group_id += 1;
        let group = TextGroupEditor::new(
            id,
            &TextGroup::default(),
            &mut self.next_text_line_id,
            &self.font_names,
            window,
            cx,
        );
        self.text_groups.push(group);
        self.refresh_preview(cx);
        cx.notify();
        self.open_text_group_editor(id, window, cx);
    }

    pub(super) fn remove_text_group(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.text_groups.iter().position(|group| group.id == id) else {
            return;
        };
        self.text_groups.remove(ix);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(super) fn add_text_line(
        &mut self,
        group_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .text_groups
            .iter_mut()
            .find(|group| group.id == group_id)
        else {
            return;
        };
        let inherited = group
            .lines
            .last()
            .map(|line| line.to_params(cx))
            .unwrap_or_default();
        let id = self.next_text_line_id;
        self.next_text_line_id += 1;
        group.lines.push(TextLine::new(
            id,
            "",
            &inherited,
            &self.font_names,
            window,
            cx,
        ));
        group.expanded_line_ids.push(id);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(super) fn set_expanded_text_lines(
        &mut self,
        group_id: u64,
        open: &[usize],
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self
            .text_groups
            .iter_mut()
            .find(|group| group.id == group_id)
        else {
            return;
        };
        group.expanded_line_ids = open
            .iter()
            .filter_map(|ix| group.lines.get(*ix).map(|line| line.id))
            .collect();
        cx.notify();
    }

    pub(super) fn remove_text_line(&mut self, id: u64, cx: &mut Context<Self>) {
        for group in &mut self.text_groups {
            let Some(ix) = group.lines.iter().position(|line| line.id == id) else {
                continue;
            };
            group.lines.remove(ix);
            group.expanded_line_ids.retain(|open| *open != id);
            self.refresh_preview(cx);
            cx.notify();
            return;
        }
    }

    pub(super) fn set_text_line_auto_color(
        &mut self,
        id: u64,
        auto_color: bool,
        cx: &mut Context<Self>,
    ) {
        for group in &mut self.text_groups {
            if let Some(line) = group.lines.iter_mut().find(|line| line.id == id) {
                line.auto_color = auto_color;
                self.refresh_preview(cx);
                cx.notify();
                return;
            }
        }
    }

    pub(super) fn set_text_line_style(
        &mut self,
        id: u64,
        bold: Option<bool>,
        italic: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        for group in &mut self.text_groups {
            if let Some(line) = group.lines.iter_mut().find(|line| line.id == id) {
                if let Some(bold) = bold {
                    line.bold = bold;
                }
                if let Some(italic) = italic {
                    line.italic = italic;
                }
                self.refresh_preview(cx);
                cx.notify();
                return;
            }
        }
    }
}
