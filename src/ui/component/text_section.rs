//! EXIF 文字组列表与独立编辑窗口。
//!
//! [`AppView`] 持有全部文字组及其控件状态；右侧参数面板只展示摘要。编辑窗口复用这些
//! 实体，并把组级参数与文字行分列展示，因此调整参数时主窗口中的照片预览仍然可见。

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, Root, Sizable as _, TitleBar,
    accordion::{Accordion, AccordionItem},
    button::{Button, ButtonVariants as _},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    searchable_list::SearchableVec,
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, IntoElement, SharedString, Subscription, Window,
    WindowBounds, WindowOptions, div, px, size,
};

use crate::media::ExifInfo;
use crate::render::text::{TIME_FORMAT_EXAMPLES, render_exif_template, time_format_is_valid};
use crate::watermark::{Placement, Text, TextAlign, TextDirection, TextGroup, TextParams};

use super::super::AppView;
use super::field::{
    Choice, ColorField, NumberField, choices, field, hint, index_of, on_select, select_state,
    warning,
};
use super::inspector::POSITIONS;

pub(super) const TEXT_ALIGNS: &[(&str, TextAlign)] = &[
    ("左对齐", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("右对齐", TextAlign::Right),
];

const HORIZONTAL_GROUP_ALIGNS: &[(&str, TextAlign)] = &[
    ("左侧", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("右侧", TextAlign::Right),
];

const VERTICAL_GROUP_ALIGNS: &[(&str, TextAlign)] = &[
    ("上侧", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("下侧", TextAlign::Right),
];

const TEXT_DIRECTIONS: &[(&str, TextDirection)] = &[
    ("横排", TextDirection::Horizontal),
    ("竖排（顺时针旋转 90°）", TextDirection::Vertical),
];

const TEMPLATE_FIELDS: &[(&str, &str)] = &[
    ("拍摄日期", "{拍摄日期}"),
    ("品牌", "{品牌}"),
    ("型号", "{型号}"),
    ("镜头型号", "{镜头型号}"),
    ("快门", "{快门}"),
    ("光圈", "{光圈}"),
    ("ISO", "{ISO}"),
    ("曝光补偿", "{曝光补偿}"),
    ("实际焦距", "{实际焦距}"),
    ("等效焦距", "{等效焦距}"),
    ("Logo", "{Logo}"),
    ("GPS", "{GPS}"),
    ("省", "{省}"),
    ("市", "{市}"),
    ("区", "{区}"),
];

pub(in crate::ui::app) type FontSelect = ComboboxState<SearchableVec<SharedString>>;
pub(in crate::ui::app) type AlignSelect = SelectState<Vec<Choice<TextAlign>>>;
pub(in crate::ui::app) type PositionSelect = SelectState<Vec<Choice<Placement>>>;
pub(in crate::ui::app) type DirectionSelect = SelectState<Vec<Choice<TextDirection>>>;

fn group_aligns(position: Placement) -> &'static [(&'static str, TextAlign)] {
    match position {
        Placement::Left | Placement::Right => VERTICAL_GROUP_ALIGNS,
        Placement::Up | Placement::Bottom | Placement::Center => HORIZONTAL_GROUP_ALIGNS,
    }
}

pub(super) struct TextGroupWindow {
    group_id: u64,
    title: SharedString,
    app: Entity<AppView>,
    _subscription: Subscription,
}

impl TextGroupWindow {
    fn new(
        group_id: u64,
        title: SharedString,
        app: Entity<AppView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            group_id,
            title,
            app,
            _subscription: subscription,
        }
    }
}

impl Render for TextGroupWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content =
            self.app
                .read(cx)
                .render_text_group_editor(self.group_id, self.app.clone(), cx);
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                TitleBar::new().child(
                    div()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(self.title.clone()),
                ),
            )
            // 这里明确建立一个有限高度的视口，右栏才有可滚动的剩余空间；外层只裁切，
            // 滚动所有权仍然属于具体的「文字行」栏。
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

pub(in crate::ui::app) struct TextLine {
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
        let font = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::new(font_items),
                vec![IndexPath::new(font_index)],
                window,
                cx,
            )
            .searchable(true)
        });
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
            if matches!(event, ComboboxEvent::Change(_) | ComboboxEvent::Confirm(_)) {
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
                .selected_values()
                .first()
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

pub(in crate::ui::app) struct TextGroupEditor {
    pub id: u64,
    pub position: Entity<PositionSelect>,
    pub align: Entity<AlignSelect>,
    pub direction: Entity<DirectionSelect>,
    pub padding: NumberField,
    pub time_format: Entity<InputState>,
    pub lines: Vec<TextLine>,
    pub expanded_line_ids: Vec<u64>,
    _subscriptions: Vec<Subscription>,
}

impl TextGroupEditor {
    pub(in crate::ui::app) fn new(
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
        let align_entries = group_aligns(group.position);
        let align = select_state(
            choices(align_entries),
            index_of(align_entries, &group.align),
            window,
            cx,
        );
        let direction = select_state(
            choices(TEXT_DIRECTIONS),
            index_of(TEXT_DIRECTIONS, &group.direction),
            window,
            cx,
        );
        let padding = NumberField::new(group.padding, 0.0, 30.0, 0.5, 1, 100.0, "%", window, cx);
        let time_format = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(group.time_format.clone())
                .placeholder("%Y/%m/%d")
        });
        let mut subscriptions = vec![on_select(&align, window, cx, |_, _, _| {})];
        let align_for_position = align.clone();
        subscriptions.push(cx.subscribe_in(
            &position,
            window,
            move |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(position)) = event else {
                    return;
                };
                let selected = align_for_position
                    .read(cx)
                    .selected_value()
                    .copied()
                    .unwrap_or_default();
                align_for_position.update(cx, |state, cx| {
                    state.set_items(choices(group_aligns(*position)), window, cx);
                    state.set_selected_value(&selected, window, cx);
                });
                this.refresh_preview(cx);
                cx.notify();
            },
        ));
        subscriptions.push(on_select(&direction, window, cx, |_, _, _| {}));
        subscriptions.extend(padding.subscribe(window, cx, |_, _| {}));
        subscriptions.push(
            cx.subscribe_in(&time_format, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );
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
            padding,
            time_format,
            lines,
            expanded_line_ids: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::ui::app) fn to_group(&self, cx: &App) -> TextGroup {
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
                .unwrap_or(Placement::Bottom),
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
            padding: self.padding.value(cx),
            time_format: self.time_format.read(cx).value().to_string(),
        }
    }
}

fn position_label(position: Placement) -> &'static str {
    match position {
        Placement::Center => "居中",
        Placement::Up => "靠上",
        Placement::Bottom => "靠下",
        Placement::Left => "靠左",
        Placement::Right => "靠右",
    }
}

fn align_label(align: TextAlign, position: Placement) -> &'static str {
    match (align, position) {
        (TextAlign::Left, Placement::Left | Placement::Right) => "上侧",
        (TextAlign::Right, Placement::Left | Placement::Right) => "下侧",
        (TextAlign::Left, _) => "左侧",
        (TextAlign::Right, _) => "右侧",
        (TextAlign::Center, _) => "居中",
    }
}

fn direction_label(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::Horizontal => "横排",
        TextDirection::Vertical => "竖排",
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
        let mut summary = format!(
            "{} · {} · {} · {} 行",
            position_label(value.position),
            align_label(value.align, value.position),
            direction_label(value.direction),
            value.text.template.len()
        );
        if !matches!(value.align, TextAlign::Center) && value.padding > 0.0 {
            summary.push_str(&format!(
                " · 文字与图片边缘留白 {:.1}%",
                value.padding * 100.0
            ));
        }
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
        _: &mut Window,
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
        if let Some(handle) = self.text_editor_windows.get(&group_id).copied() {
            if handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
            self.text_editor_windows.remove(&group_id);
        }
        if !self.opening_text_editor_ids.insert(group_id) {
            return;
        }

        let view = cx.entity();
        let title: SharedString = format!("文字组 {}", ix + 1).into();
        cx.defer(move |cx| {
            let should_open = {
                let app = view.read(cx);
                app.opening_text_editor_ids.contains(&group_id)
                    && app.text_groups.iter().any(|group| group.id == group_id)
            };
            if !should_open {
                return;
            }
            let bounds = Bounds::centered(None, size(px(960.), px(720.)), cx);
            let editor_view = view.clone();
            let editor_title = title.clone();
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(760.), px(560.))),
                    ..TitleBar::window_options()
                },
                move |window, cx| {
                    let editor =
                        cx.new(|cx| TextGroupWindow::new(group_id, editor_title, editor_view, cx));
                    cx.new(|cx| Root::new(editor, window, cx))
                },
            );
            view.update(cx, |this, cx| {
                this.opening_text_editor_ids.remove(&group_id);
                match result {
                    Ok(handle) => {
                        this.text_editor_windows.insert(group_id, handle);
                    }
                    Err(error) => {
                        eprintln!("无法打开文字组编辑窗口: {error:#}");
                    }
                }
                cx.notify();
            });
        });
    }

    fn render_text_group_editor(
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
        let time_input = group.time_format.clone();
        let time_view = view.clone();
        // 居中没有单一的相邻边框；只有贴边对齐时才显示文字与那一侧边框的空白。
        let show_padding = matches!(value.align, TextAlign::Left | TextAlign::Right);

        h_flex()
            .id(("text-group-editor", group_id))
            .items_stretch()
            .size_full()
            .min_h_0()
            .w_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .id(("text-group-settings", group_id))
                    .w_80()
                    .min_h_0()
                    .flex_shrink_0()
                    .gap_4()
                    .p_4()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .overflow_y_scroll()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("文字组设置"),
                    )
                    .child(field("位置", Select::new(&group.position).w_full(), cx))
                    .child(field("组对齐", Select::new(&group.align).w_full(), cx))
                    .child(
                        group
                            .padding
                            .render("文字与图片边缘留白", !show_padding, cx),
                    )
                    .child(field("方向", Select::new(&group.direction).w_full(), cx))
                    .child(field(
                        "时间格式",
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&group.time_format)),
                            )
                            .child(
                                Button::new(("time-format-options", group_id))
                                    .label("常用格式")
                                    .dropdown_caret(true)
                                    .outline()
                                    .small()
                                    .dropdown_menu(move |menu, _, _| {
                                        TIME_FORMAT_EXAMPLES.iter().fold(
                                            menu,
                                            |menu, (label, value)| {
                                                let input = time_input.clone();
                                                let view = time_view.clone();
                                                menu.item(PopupMenuItem::new(*label).on_click(
                                                    move |_, window, cx| {
                                                        view.update(cx, |this, cx| {
                                                            this.set_input_value(
                                                                &input, value, window, cx,
                                                            )
                                                        });
                                                    },
                                                ))
                                            },
                                        )
                                    }),
                            ),
                        cx,
                    ))
                    .child(if time_format_is_valid(&value.time_format) {
                        div().into_any_element()
                    } else {
                        warning("时间格式无效，将使用默认的 %Y/%m/%d。", cx)
                    }),
            )
            .child(
                v_flex()
                    .id(("text-group-lines", group_id))
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .gap_4()
                    .p_4()
                    .overflow_y_scrollbar()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child("文字行"),
                            )
                            .child(
                                Button::new(("text-add-line", group_id))
                                    .icon(IconName::Plus)
                                    .label("添加一行")
                                    .on_click(move |_, window, cx| {
                                        add_view.update(cx, |this, cx| {
                                            this.add_text_line(group_id, window, cx)
                                        });
                                    }),
                            ),
                    )
                    .when(group.lines.is_empty(), |this| {
                        this.child(hint("这个文字组还没有文字行。", cx))
                    })
                    .when(!group.lines.is_empty(), |this| {
                        this.child(lines.on_toggle_click(move |open: &[usize], _, cx| {
                            toggle_view.update(cx, |this, cx| {
                                this.set_expanded_text_lines(group_id, open, cx)
                            });
                        }))
                    }),
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
        let italic_view = view.clone();
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
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&line.template)))
                    .child(self.render_template_field_menu(
                        line.template.clone(),
                        view.clone(),
                        id,
                    )),
            )
            .child(resolved)
            .child(line.size.render("字号", false, cx))
            .child(line.line_spacing.render("行距", false, cx))
            .child(field(
                "字体",
                Combobox::new(&line.font)
                    .search_placeholder("搜索字体…")
                    .w_full(),
                cx,
            ))
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
                    .pt_2()
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

    fn render_template_field_menu(
        &self,
        input: Entity<InputState>,
        view: Entity<AppView>,
        line_id: u64,
    ) -> impl IntoElement {
        Button::new(("template-field-options", line_id))
            .label("插入字段")
            .dropdown_caret(true)
            .outline()
            .small()
            .dropdown_menu(move |menu, _, _| {
                TEMPLATE_FIELDS.iter().fold(menu, |menu, (label, value)| {
                    let input = input.clone();
                    let view = view.clone();
                    menu.item(PopupMenuItem::new(*label).on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.append_to_input(&input, value, window, cx)
                        });
                    }))
                })
            })
    }

    fn set_input_value(
        &mut self,
        input: &Entity<InputState>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |state, cx| state.set_value(value, window, cx));
        self.refresh_preview(cx);
        cx.notify();
    }

    fn append_to_input(
        &mut self,
        input: &Entity<InputState>,
        suffix: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = format!("{}{}", input.read(cx).value(), suffix);
        self.set_input_value(input, &value, window, cx);
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
        self.opening_text_editor_ids.remove(&id);
        if let Some(handle) = self.text_editor_windows.remove(&id) {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_alignment_labels_follow_group_position() {
        assert_eq!(
            group_aligns(Placement::Up),
            &[
                ("左侧", TextAlign::Left),
                ("居中", TextAlign::Center),
                ("右侧", TextAlign::Right),
            ]
        );
        assert_eq!(
            group_aligns(Placement::Right),
            &[
                ("上侧", TextAlign::Left),
                ("居中", TextAlign::Center),
                ("下侧", TextAlign::Right),
            ]
        );
        assert_eq!(align_label(TextAlign::Left, Placement::Left), "上侧");
        assert_eq!(align_label(TextAlign::Right, Placement::Right), "下侧");
        assert_eq!(align_label(TextAlign::Left, Placement::Bottom), "左侧");
        assert_eq!(align_label(TextAlign::Right, Placement::Up), "右侧");
    }
}
