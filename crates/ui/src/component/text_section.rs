//! EXIF 文字组列表与独立编辑窗口。
//!
//! [`AppView`] 持有全部文字组及其控件状态；右侧参数面板只展示摘要。编辑窗口复用这些
//! 实体，并把组级参数与文字行分列展示，因此调整参数时主窗口中的照片预览仍然可见。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Selectable as _, Sizable as _,
    TitleBar,
    accordion::{Accordion, AccordionItem},
    button::{Button, ButtonVariants as _},
    combobox::{ComboboxEvent, ComboboxState},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputEvent, InputState, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    searchable_list::SearchableVec,
    select::SelectState,
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, IntoElement, SharedString, Subscription, Window,
    WindowBounds, WindowOptions, div, px, size,
};

use crate::backend::media::ExifInfo;
use crate::backend::render::text::{TIME_FORMAT_EXAMPLES, time_format_is_valid};
use crate::backend::watermark::{
    GroupAttachment, PhotoAnchor, PhotoPlacement, Placement, Text, TextAlign, TextDirection,
    TextGroup, TextParams,
};

use super::super::AppView;
use super::field::{
    Choice, ColorField, NumberField, choices, description, field, index_of, muted_text, on_select,
    select_state, warning,
};
use super::inspector::POSITIONS;
use super::rich_text::{RichTextEditor, token_content, toolbar_button};
use super::text_border::TextBorderControls;

pub(super) const TEXT_ALIGNS: &[(&str, TextAlign)] = &[
    ("左对齐", TextAlign::Left),
    ("居中", TextAlign::Center),
    ("右对齐", TextAlign::Right),
];

const PHOTO_ANCHORS: &[(&str, &str, &str, Option<PhotoAnchor>)] = &[
    (
        "text-photo-top-left",
        "↖",
        "左上",
        Some(PhotoAnchor::TopLeft),
    ),
    ("text-photo-top", "↑", "上方", Some(PhotoAnchor::Top)),
    (
        "text-photo-top-right",
        "↗",
        "右上",
        Some(PhotoAnchor::TopRight),
    ),
    ("text-photo-left", "←", "左侧", Some(PhotoAnchor::Left)),
    ("text-photo-center", "", "", None),
    ("text-photo-right", "→", "右侧", Some(PhotoAnchor::Right)),
    (
        "text-photo-bottom-left",
        "↙",
        "左下",
        Some(PhotoAnchor::BottomLeft),
    ),
    ("text-photo-bottom", "↓", "下方", Some(PhotoAnchor::Bottom)),
    (
        "text-photo-bottom-right",
        "↘",
        "右下",
        Some(PhotoAnchor::BottomRight),
    ),
];

fn photo_anchor_label(anchor: PhotoAnchor) -> &'static str {
    PHOTO_ANCHORS
        .iter()
        .find(|(_, _, _, value)| *value == Some(anchor))
        .unwrap()
        .2
}

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
    ("自定义文本", "{自定义文本}"),
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
    ("海拔", "{海拔}"),
    ("省", "{省}"),
    ("市", "{市}"),
    ("区", "{区}"),
];

pub(in crate::app) type FontSelect = ComboboxState<SearchableVec<SharedString>>;
pub(in crate::app) type AlignSelect = SelectState<Vec<Choice<TextAlign>>>;
pub(in crate::app) type DirectionSelect = SelectState<Vec<Choice<TextDirection>>>;

pub(super) const INHERIT_FONT_LABEL: &str = "跟随默认字体";

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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content =
            self.app
                .read(cx)
                .render_text_group_editor(self.group_id, self.app.clone(), cx);
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                crate::window_effects::title_bar(cx).child(
                    div()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(self.title.clone()),
                ),
            )
            // 这里明确建立一个有限高度的视口，右栏才有可滚动的剩余空间；外层只裁切，
            // 滚动所有权仍然属于具体的「文字行」栏。
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
    }
}

pub(in crate::app) struct TextLine {
    pub id: u64,
    pub template: Entity<TextareaState>,
    pub(super) rich: RichTextEditor,
    pub size: NumberField,
    pub logo_size: NumberField,
    pub line_spacing: NumberField,
    pub font: Entity<FontSelect>,
    pub align: Entity<AlignSelect>,
    pub color: ColorField,
    pub auto_color: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub border: TextBorderControls,
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
            let mut state = TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder("例如 {Logo} {型号}");
            state.set_value(token_content(template, None), window, cx);
            state
        });
        let size = NumberField::new(params.size, 1.0, 12.0, 0.1, 1, 100.0, "%", window, cx);
        let logo_size = NumberField::new(
            params.logo_size.unwrap_or(params.size),
            0.5,
            25.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let line_spacing =
            NumberField::new(params.line_spacing, 1.0, 2.5, 0.05, 2, 1.0, "", window, cx);

        let configured = if params.font.trim().is_empty() {
            SharedString::from(INHERIT_FONT_LABEL)
        } else {
            SharedString::from(params.font.clone())
        };
        let mut font_items = Vec::with_capacity(fonts.len() + 2);
        font_items.push(INHERIT_FONT_LABEL.into());
        if configured.as_ref() != INHERIT_FONT_LABEL
            && !fonts.iter().any(|font| font == &configured)
        {
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
        let border =
            TextBorderControls::new(&params.border.clone().unwrap_or_default(), window, cx);

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &template_input,
            window,
            move |this, state, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rich_text_changed(id, &state.read(cx).value(), cx);
                    this.refresh_preview(cx);
                    cx.notify();
                } else if matches!(event, InputEvent::Blur) {
                    this.finish_rich_token_edit(id, window, cx);
                }
            },
        ));
        // 模板在独立编辑窗口中显示，不能把选区同步绑定到创建控件的主窗口。
        subscriptions.push(cx.observe(&template_input, move |_, _, cx| {
            let view = cx.entity().downgrade();
            cx.defer(move |cx| {
                for window in cx.windows() {
                    window
                        .update(cx, |_, window, cx| {
                            view.update(cx, |this, cx| this.sync_rich_toolbar(id, window, cx))
                                .ok();
                        })
                        .ok();
                }
            });
        }));
        subscriptions.extend(size.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(logo_size.subscribe(window, cx, |_, _| {}));
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
            rich: RichTextEditor::new(id, template, params, fonts, window, cx),
            size,
            logo_size,
            line_spacing,
            font,
            align,
            color,
            auto_color: params.color.is_none(),
            bold: params.bold,
            italic: params.italic,
            underline: params.underline,
            border,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn to_params(&self, cx: &App) -> TextParams {
        let selected_font = self
            .font
            .read(cx)
            .selected_values()
            .first()
            .cloned()
            .unwrap_or_default()
            .to_string();
        TextParams {
            border: self.border.to_optional_border(cx),
            spans: self.rich.spans.clone(),
            font: if selected_font == INHERIT_FONT_LABEL {
                String::new()
            } else {
                selected_font
            },
            size: self.size.value(cx),
            logo_size: Some(self.logo_size.value(cx)),
            line_spacing: self.line_spacing.value(cx),
            color: if self.auto_color {
                None
            } else {
                Some(self.color.value(cx))
            },
            italic: self.italic,
            underline: self.underline,
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

pub(in crate::app) struct TextGroupEditor {
    pub id: u64,
    pub position: Placement,
    pub align: TextAlign,
    pub photo_anchor: Option<PhotoAnchor>,
    pub photo_inset: NumberField,
    pub direction: Entity<DirectionSelect>,
    pub padding: NumberField,
    pub border: TextBorderControls,
    pub background_enabled: bool,
    pub background_color: ColorField,
    pub time_format: Entity<InputState>,
    pub lines: Vec<TextLine>,
    pub expanded_line_ids: Vec<u64>,
    pub attachment_target: Option<u64>,
    pub attachment_side: Placement,
    pub attachment_gap: NumberField,
    _subscriptions: Vec<Subscription>,
}

impl TextGroupEditor {
    pub(in crate::app) fn new(
        id: u64,
        group: &TextGroup,
        next_line_id: &mut u64,
        fonts: &[SharedString],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let direction = select_state(
            choices(TEXT_DIRECTIONS),
            index_of(TEXT_DIRECTIONS, &group.direction),
            window,
            cx,
        );
        let padding = NumberField::new(group.padding, 0.0, 30.0, 0.5, 1, 100.0, "%", window, cx);
        let photo_inset = NumberField::new(
            group
                .photo_placement
                .as_ref()
                .map_or(0.02, PhotoPlacement::inset),
            0.0,
            30.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let border = TextBorderControls::new(&group.border, window, cx);
        let background_color =
            ColorField::new_rgba(group.background.unwrap_or([0, 0, 0, 255]), window, cx);
        let attachment_side = group
            .attachment
            .as_ref()
            .map(|a| a.side)
            .unwrap_or(Placement::Left);
        let attachment_gap = NumberField::new(
            group.attachment.as_ref().map(|a| a.gap).unwrap_or(0.0),
            0.0,
            20.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let time_format = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(group.time_format.clone())
                .placeholder("%Y/%m/%d")
        });
        let mut subscriptions = Vec::new();
        subscriptions.push(on_select(&direction, window, cx, |_, _, _| {}));
        subscriptions.extend(padding.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(background_color.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(photo_inset.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(attachment_gap.subscribe(window, cx, |_, _| {}));
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
            position: group.position,
            align: group.align,
            photo_anchor: group.photo_placement.as_ref().map(PhotoPlacement::anchor),
            photo_inset,
            direction,
            padding,
            border,
            background_enabled: group.background.is_some(),
            background_color,
            time_format,
            lines,
            expanded_line_ids: Vec::new(),
            attachment_target: None,
            attachment_side,
            attachment_gap,
            _subscriptions: subscriptions,
        }
    }

    fn render_background(&self, view: Entity<AppView>, cx: &App) -> impl IntoElement {
        let id = self.id;
        v_flex()
            .w_full()
            .gap_3()
            .child(
                Switch::new(("text-group-background", id))
                    .label("文字组背景")
                    .checked(self.background_enabled)
                    .on_change(move |enabled, _, cx| {
                        view.update(cx, |this, cx| {
                            if let Some(group) =
                                this.text_groups.iter_mut().find(|group| group.id == id)
                            {
                                group.background_enabled = *enabled;
                                this.refresh_preview(cx);
                                cx.notify();
                            }
                        });
                    }),
            )
            .when(self.background_enabled, |this| {
                this.child(self.background_color.render("背景颜色", false, cx))
                    .child(description(
                        "在颜色选择器中调整不透明度，或输入 #RRGGBBAA；自动文字颜色会考虑叠加后的背景。",
                    ))
            })
    }

    pub(in crate::app) fn attachment(&self, target: usize, cx: &App) -> GroupAttachment {
        GroupAttachment {
            target,
            side: self.attachment_side,
            gap: self.attachment_gap.value(cx),
        }
    }

    pub(in crate::app) fn to_group(&self, cx: &App) -> TextGroup {
        TextGroup {
            text: Text {
                template: self
                    .lines
                    .iter()
                    .map(|line| line.template.read(cx).value().to_string())
                    .collect(),
                text_params: self.lines.iter().map(|line| line.to_params(cx)).collect(),
            },
            position: self.position,
            direction: self
                .direction
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            align: self.align,
            photo_placement: self.photo_anchor.map(|anchor| {
                let mut inside = PhotoPlacement::new(anchor);
                inside.set_inset(self.photo_inset.value(cx));
                inside
            }),
            padding: self.padding.value(cx),
            border: self.border.to_border(cx),
            background: self
                .background_enabled
                .then(|| self.background_color.rgba_value(cx)),
            time_format: self.time_format.read(cx).value().to_string(),
            attachment: None,
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
            .title(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child("文字与 Logo 水印")
                    .child(
                        Button::new("custom-logos-open")
                            .icon(gpui_kit::assets::IconName::Images)
                            .label("自定义 Logo…")
                            .outline()
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_logo_window(window, cx)
                                }),
                            ),
                    ),
            )
            .child(description("每个文字组可独立设置位置、方向和多行文字。"))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .when(self.text_groups.is_empty(), |this| {
                        this.child(muted_text("还没有文字组。", cx))
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
        if let Some(inside) = &value.photo_placement
            && group.attachment_target.is_none()
        {
            summary = format!(
                "照片内{} · {} · {} 行",
                photo_anchor_label(inside.anchor()),
                direction_label(value.direction),
                value.text.template.len()
            );
        }
        if value.photo_placement.is_none()
            && !matches!(value.align, TextAlign::Center)
            && value.padding > 0.0
        {
            summary.push_str(&format!(
                " · 文字与图片边缘留白 {:.1}%",
                value.padding * 100.0
            ));
        }
        let title = value
            .text
            .template
            .iter()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().chars().take(30).collect::<String>())
            .unwrap_or_else(|| format!("空文字组 {}", ix + 1));
        let edit_view = view.clone();
        v_flex()
            .id(("text-group-row", id))
            .w_full()
            .gap_1()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new(("text-group-edit", id))
                            .label(title)
                            .ghost()
                            .small()
                            .justify_start()
                            .flex_1()
                            .min_w_0()
                            .tooltip("编辑文字组…")
                            .accessibility_label(format!("编辑文字组 {}", ix + 1))
                            .on_click(move |_, window, cx| {
                                edit_view.update(cx, |this, cx| {
                                    this.open_text_group_editor(id, window, cx)
                                })
                            }),
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
            .child(div().px_2().child(muted_text(summary, cx)))
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
            let result = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(760.), px(560.))),
                    ..TitleBar::window_options()
                },
                cx,
                move |_, cx| {
                    cx.new(|cx| TextGroupWindow::new(group_id, editor_title, editor_view, cx))
                },
            );
            view.update(cx, |this, cx| {
                this.opening_text_editor_ids.remove(&group_id);
                match result {
                    Ok((handle, _)) => {
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

    fn can_attach_group(&self, id: u64, target: u64) -> bool {
        let mut current = Some(target);
        let mut seen = std::collections::HashSet::new();
        while let Some(target) = current {
            if target == id || !seen.insert(target) {
                return false;
            }
            current = self
                .text_groups
                .iter()
                .find(|group| group.id == target)
                .and_then(|group| group.attachment_target);
        }
        true
    }

    fn set_group_attachment(
        &mut self,
        id: u64,
        target: Option<u64>,
        side: Placement,
        cx: &mut Context<Self>,
    ) {
        if target.is_some_and(|target| !self.can_attach_group(id, target)) {
            return;
        }
        if let Some(group) = self.text_groups.iter_mut().find(|group| group.id == id) {
            group.attachment_target = target;
            group.attachment_side = side;
            self.refresh_preview(cx);
            cx.notify();
        }
    }

    fn render_attachment_settings(
        &self,
        group: &TextGroupEditor,
        view: Entity<AppView>,
        cx: &App,
    ) -> AnyElement {
        let id = group.id;
        let target = group.attachment_target;
        let side = group.attachment_side;
        let target_label = target
            .and_then(|target| self.text_groups.iter().position(|group| group.id == target))
            .map(|ix| format!("文字组 {}", ix + 1))
            .unwrap_or_else(|| "独立位置".into());
        let targets = self
            .text_groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.id != id)
            .map(|(ix, group)| {
                (
                    group.id,
                    format!("文字组 {}", ix + 1),
                    self.can_attach_group(id, group.id),
                )
            })
            .collect::<Vec<_>>();
        let target_view = view.clone();
        let side_view = view.clone();
        v_flex()
            .gap_4()
            .w_full()
            .child(field(
                "紧贴文字组",
                Button::new(("attachment-target", id))
                    .label(target_label)
                    .outline()
                    .dropdown_caret(true)
                    .w_full()
                    .dropdown_menu(move |menu, _, _| {
                        let view = target_view.clone();
                        let menu = menu.item(
                            PopupMenuItem::new("独立位置")
                                .checked(target.is_none())
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.set_group_attachment(id, None, side, cx)
                                    });
                                }),
                        );
                        targets
                            .iter()
                            .fold(menu, |menu, (target_id, label, enabled)| {
                                let target_id = *target_id;
                                let view = target_view.clone();
                                menu.item(
                                    PopupMenuItem::new(label.clone())
                                        .checked(target == Some(target_id))
                                        .disabled(!enabled)
                                        .on_click(move |_, _, cx| {
                                            view.update(cx, |this, cx| {
                                                this.set_group_attachment(
                                                    id,
                                                    Some(target_id),
                                                    side,
                                                    cx,
                                                )
                                            });
                                        }),
                                )
                            })
                    }),
                cx,
            ))
            .when(target.is_some(), |this| {
                this.child(field(
                    "相邻方向",
                    Button::new(("attachment-side", id))
                        .label(match side {
                            Placement::Right => "右侧",
                            Placement::Up => "上方",
                            Placement::Bottom => "下方",
                            _ => "左侧",
                        })
                        .outline()
                        .dropdown_caret(true)
                        .w_full()
                        .dropdown_menu(move |menu, _, _| {
                            [
                                ("左侧", Placement::Left),
                                ("右侧", Placement::Right),
                                ("上方", Placement::Up),
                                ("下方", Placement::Bottom),
                            ]
                            .into_iter()
                            .fold(menu, |menu, (label, value)| {
                                let view = side_view.clone();
                                menu.item(
                                    PopupMenuItem::new(label).checked(side == value).on_click(
                                        move |_, _, cx| {
                                            view.update(cx, |this, cx| {
                                                this.set_group_attachment(id, target, value, cx)
                                            });
                                        },
                                    ),
                                )
                            })
                        }),
                    cx,
                ))
                .child(group.attachment_gap.render("组间距", false, cx))
                .child(description("间距为 0 时紧贴目标组；位置和对齐跟随目标组。"))
            })
            .into_any_element()
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
        let direction = group.direction.clone();
        let direction_view = view.clone();
        // 居中没有单一的相邻边框；只有贴边对齐时才显示文字与那一侧边框的空白。
        let show_padding = group.attachment_target.is_none()
            && matches!(value.align, TextAlign::Left | TextAlign::Right);

        h_flex()
            .id(("text-group-editor", group_id))
            .items_stretch()
            .size_full()
            .min_h_0()
            .w_full()
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
                    .child(self.render_attachment_settings(group, view.clone(), cx))
                    .when(group.attachment_target.is_none(), |this| {
                        let view = view.clone();
                        this.child(field(
                            "放置区域",
                            TabBar::new(("text-group-placement-mode", group_id))
                                .segmented()
                                .w_full()
                                .children(
                                    ["画布位置", "照片内部"]
                                        .into_iter()
                                        .map(|label| Tab::new().label(label).flex_1()),
                                )
                                .selected_index(usize::from(group.photo_anchor.is_some()))
                                .on_click(move |index: &usize, _, cx| {
                                    view.update(cx, |this, cx| {
                                        if let Some(group) = this
                                            .text_groups
                                            .iter_mut()
                                            .find(|group| group.id == group_id)
                                        {
                                            group.photo_anchor = if *index == 1 {
                                                Some(
                                                    group
                                                        .photo_anchor
                                                        .unwrap_or(PhotoAnchor::BottomRight),
                                                )
                                            } else {
                                                None
                                            };
                                            this.refresh_preview(cx);
                                            cx.notify();
                                        }
                                    });
                                }),
                            cx,
                        ))
                    })
                    .when(
                        group.attachment_target.is_none() && group.photo_anchor.is_some(),
                        |this| {
                            this.child(field(
                                "内部位置",
                                v_flex()
                                    .gap_1()
                                    .children(PHOTO_ANCHORS.chunks(3).map(|row| {
                                        h_flex().gap_1().children(row.iter().map(
                                            |&(key, arrow, label, anchor)| {
                                                let view = view.clone();
                                                Button::new((key, group_id))
                                                    .label(arrow)
                                                    .size_9()
                                                    .outline()
                                                    .disabled(anchor.is_none())
                                                    .selected(
                                                        anchor.is_some()
                                                            && anchor == group.photo_anchor,
                                                    )
                                                    .tooltip(format!("照片内{label}"))
                                                    .accessibility_label(format!(
                                                        "放在照片内{label}"
                                                    ))
                                                    .on_click(move |_, _, cx| {
                                                        if anchor.is_none() {
                                                            return;
                                                        }
                                                        view.update(cx, |this, cx| {
                                                            if let Some(group) = this
                                                                .text_groups
                                                                .iter_mut()
                                                                .find(|group| group.id == group_id)
                                                            {
                                                                group.photo_anchor = anchor;
                                                                this.refresh_preview(cx);
                                                                cx.notify();
                                                            }
                                                        });
                                                    })
                                            },
                                        ))
                                    })),
                                cx,
                            ))
                            .child(group.photo_inset.render("内侧留白", false, cx))
                            .child(description(
                                "按照片高度计算内侧留白；照片内的文字组不会增加外侧边框。",
                            ))
                        },
                    )
                    .when(
                        group.attachment_target.is_none() && group.photo_anchor.is_none(),
                        |this| {
                            let view = view.clone();
                            this.child(field(
                                "位置",
                                TabBar::new(("text-group-position", group_id))
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
                                            .position(|(_, position)| *position == value.position)
                                            .unwrap_or(0),
                                    )
                                    .on_click(move |index: &usize, _, cx| {
                                        view.update(cx, |this, cx| {
                                            if let Some(group) = this
                                                .text_groups
                                                .iter_mut()
                                                .find(|group| group.id == group_id)
                                            {
                                                group.position = POSITIONS[*index].1;
                                                this.refresh_preview(cx);
                                                cx.notify();
                                            }
                                        });
                                    }),
                                cx,
                            ))
                        },
                    )
                    .when(
                        group.attachment_target.is_none() && group.photo_anchor.is_none(),
                        |this| {
                            let view = view.clone();
                            let aligns = group_aligns(value.position);
                            this.child(field(
                                "组对齐",
                                TabBar::new(("text-group-align", group_id))
                                    .segmented()
                                    .w_full()
                                    .children(
                                        aligns
                                            .iter()
                                            .map(|(label, _)| Tab::new().label(*label).flex_1()),
                                    )
                                    .selected_index(
                                        aligns
                                            .iter()
                                            .position(|(_, align)| *align == value.align)
                                            .unwrap_or(1),
                                    )
                                    .on_click(move |index: &usize, _, cx| {
                                        view.update(cx, |this, cx| {
                                            if let Some(group) = this
                                                .text_groups
                                                .iter_mut()
                                                .find(|group| group.id == group_id)
                                            {
                                                group.align = aligns[*index].1;
                                                this.refresh_preview(cx);
                                                cx.notify();
                                            }
                                        });
                                    }),
                                cx,
                            ))
                        },
                    )
                    .when(group.photo_anchor.is_none(), |this| {
                        this.child(
                            group
                                .padding
                                .render("文字与图片边缘留白", !show_padding, cx),
                        )
                    })
                    .child(field(
                        "方向",
                        TabBar::new(("text-group-direction", group_id))
                            .segmented()
                            .w_full()
                            .children(TEXT_DIRECTIONS.iter().map(|(_, direction)| {
                                Tab::new().label(direction_label(*direction)).flex_1()
                            }))
                            .selected_index(
                                TEXT_DIRECTIONS
                                    .iter()
                                    .position(|(_, direction)| *direction == value.direction)
                                    .unwrap_or(0),
                            )
                            .on_click(move |index: &usize, window, cx| {
                                direction_view.update(cx, |this, cx| {
                                    direction.update(cx, |state, cx| {
                                        state.set_selected_value(
                                            &TEXT_DIRECTIONS[*index].1,
                                            window,
                                            cx,
                                        );
                                    });
                                    this.refresh_preview(cx);
                                    cx.notify();
                                });
                            }),
                        cx,
                    ))
                    .child(group.render_background(view.clone(), cx))
                    .child(group.border.render(
                        group_id,
                        group.background_enabled,
                        view.clone(),
                        cx,
                    ))
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
                        this.child(muted_text("这个文字组还没有文字行。", cx))
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
        let align_view = view.clone();
        let align = line.align.clone();
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
        let has_logo = crate::backend::logos::template_logo_names(&template)
            .next()
            .is_some();
        let empty_exif = ExifInfo::default();
        let exif =
            exif.or_else(|| (template.contains("{自定义文本}") || has_logo).then_some(&empty_exif));
        let resolved = self.render_rich_text_result(line, exif, time_format, cx);
        let open = self
            .text_groups
            .iter()
            .find(|group| group.id == group_id)
            .is_some_and(|group| group.expanded_line_ids.contains(&id));

        item.open(open)
            .title(title)
            .child(self.render_rich_composer(line, view.clone(), cx))
            .child(resolved)
            .child(line.size.render("字号", false, cx))
            .when(has_logo, |this| {
                this.child(line.logo_size.render("Logo 高度", false, cx))
                    .child(description("按照片高度计算，仅缩放 Logo 的可见内容。"))
            })
            .child(line.line_spacing.render("行距", false, cx))
            .child(field(
                "行对齐",
                TabBar::new(("text-line-align", id))
                    .segmented()
                    .w_full()
                    .children(
                        TEXT_ALIGNS
                            .iter()
                            .map(|(label, _)| Tab::new().label(*label).flex_1()),
                    )
                    .selected_index(
                        TEXT_ALIGNS
                            .iter()
                            .position(|(_, value)| {
                                Some(value) == line.align.read(cx).selected_value()
                            })
                            .unwrap_or(1),
                    )
                    .on_click(move |index: &usize, window, cx| {
                        align_view.update(cx, |this, cx| {
                            align.update(cx, |state, cx| {
                                state.set_selected_value(&TEXT_ALIGNS[*index].1, window, cx);
                            });
                            this.refresh_preview(cx);
                            cx.notify();
                        });
                    }),
                cx,
            ))
            .child(line.border.render_line(id, view, cx))
    }

    pub(super) fn render_template_field_menu(
        &self,
        input: Entity<TextareaState>,
        view: Entity<AppView>,
        line_id: u64,
    ) -> impl IntoElement {
        let logo_fields = self
            .logos
            .assets
            .iter()
            .map(|asset| (asset.name(), asset.token()))
            .collect::<Vec<_>>();
        toolbar_button(
            ("template-field-options", line_id),
            gpui_kit::assets::IconName::Braces,
            "插入字段",
        )
        .dropdown_menu(move |menu, _, _| {
            let menu = TEMPLATE_FIELDS.iter().fold(menu, |menu, (label, value)| {
                let input = input.clone();
                let view = view.clone();
                menu.item(PopupMenuItem::new(*label).on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        this.insert_template_field(&input, value, window, cx)
                    });
                }))
            });
            logo_fields.iter().fold(menu, |menu, (label, value)| {
                let input = input.clone();
                let view = view.clone();
                let value = value.clone();
                menu.item(
                    PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.insert_template_field(&input, &value, window, cx)
                        });
                    }),
                )
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

    fn insert_template_field(
        &mut self,
        input: &Entity<TextareaState>,
        suffix: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::base::input::InlineToken;
        input.update(cx, |state, cx| {
            let label = suffix.trim_start_matches('{').trim_end_matches('}');
            state
                .replace_with_token(
                    InlineToken::new(format!("field:{suffix}"), suffix.to_owned())
                        .with_label(label.to_owned()),
                    window,
                    cx,
                )
                .ok();
            state.focus(window, cx);
        });
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
        if let Some(mut attachment) = self.backend.params().palette_card.attachment().cloned() {
            if attachment.target == ix {
                self.backend.params_mut().palette_card.set_attachment(None);
            } else if attachment.target > ix {
                attachment.target -= 1;
                self.backend
                    .params_mut()
                    .palette_card
                    .set_attachment(Some(attachment));
            }
        }
        for group in &mut self.text_groups {
            if group.attachment_target == Some(id) {
                group.attachment_target = None;
            }
        }
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
        let mut inherited = group
            .lines
            .last()
            .map(|line| line.to_params(cx))
            .unwrap_or_default();
        inherited.spans.clear();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn group_buttons_keep_position_alignment_and_border_settings(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::component::Root;
        use gpui_kit::test::TestWindowExt as _;
        cx.update(gpui_kit::init);
        let (root, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = AppView::new_with_settings_path(None, window, cx);
                let line_id = app.text_groups[0].lines[0].id;
                app.text_groups[0].expanded_line_ids = vec![line_id];
                app
            });
            let editor = cx.new(|cx| TextGroupWindow::new(0, "文字组".into(), app, cx));
            Root::new(editor, window, cx)
        });
        let editor = root.read_with(cx, |root, _| {
            root.view()
                .clone()
                .downcast::<TextGroupWindow>()
                .ok()
                .unwrap()
        });
        let app = editor.read_with(cx, |editor, _| editor.app.clone());
        cx.simulate_resize(size(px(1280.), px(1000.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window
                .within(("text-group-position", 0_u64))
                .click(3_usize, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window
                .within(("text-group-align", 0_u64))
                .click(2_usize, cx)
        });
        cx.run_until_parked();
        let line_id = app.read_with(cx, |app, _| app.text_groups[0].lines[0].id);
        cx.update(|window, cx| {
            window
                .within(("text-group-direction", 0_u64))
                .click(1_usize, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window
                .within(("text-line-align", line_id))
                .click(0_usize, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.click(
                (
                    gpui_kit::ElementId::from(("text-line-border", line_id)),
                    "text-border-enabled",
                ),
                cx,
            );
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.click(("text-group-background", 0_u64), cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.click(("text-border-enabled", 0_u64), cx));
        cx.run_until_parked();
        for key in ["text-border-top", "text-border-right", "text-border-bottom"] {
            cx.update(|window, cx| window.click((key, 0_u64), cx));
            cx.run_until_parked();
        }
        cx.update(|window, cx| {
            window
                .within(("text-group-placement-mode", 0_u64))
                .click(1_usize, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.click(("text-photo-top-left", 0_u64), cx));
        cx.run_until_parked();
        app.update_in(cx, |app, window, cx| {
            let snapshot = app.current_watermark(cx);
            let inside = snapshot.text_groups[0].photo_placement.as_ref().unwrap();
            assert_eq!(inside.anchor(), PhotoAnchor::TopLeft);
            assert!((inside.inset() - 0.02).abs() < 1e-8);
            assert_eq!(snapshot.text_groups[0].position, Placement::Left);
            assert_eq!(snapshot.text_groups[0].align, TextAlign::Right);
            assert_eq!(snapshot.text_groups[0].direction, TextDirection::Vertical);
            assert_eq!(
                snapshot.text_groups[0].text.text_params[0].align,
                TextAlign::Left
            );
            assert_eq!(snapshot.text_groups[0].background, Some([0, 0, 0, 255]));
            let border = &snapshot.text_groups[0].border;
            assert!(
                snapshot.text_groups[0].text.text_params[0]
                    .border
                    .as_ref()
                    .unwrap()
                    .enabled()
            );
            assert!(
                snapshot.text_groups[0].text.text_params[1].border.is_none(),
                "{:?}",
                snapshot.text_groups[0].text.text_params[1].border
            );
            assert!(border.enabled());
            assert_eq!(border.color(), [192; 3]);
            assert!(border.side(Placement::Left));
            for side in [Placement::Up, Placement::Right, Placement::Bottom] {
                assert!(!border.side(side));
            }
            app.add_text_group(window, cx);
            assert!(!app.build_text_groups(cx)[1].border.enabled());
            let mut snapshot = snapshot;
            snapshot.text_groups[0].background = Some([12, 34, 56, 128]);
            app.restore_watermark(snapshot.clone(), window, cx);
            assert_eq!(
                app.build_text_groups(cx)[0].background,
                Some([12, 34, 56, 128])
            );
            assert_eq!(app.text_groups[0].photo_anchor, Some(PhotoAnchor::TopLeft));
            assert_eq!(
                app.build_text_groups(cx)[0].border,
                snapshot.text_groups[0].border
            );
            assert_eq!(
                app.build_text_groups(cx)[0].text.text_params[0].border,
                snapshot.text_groups[0].text.text_params[0].border
            );
            assert_eq!(app.build_text_groups(cx)[0].position, Placement::Left);
            assert_eq!(app.build_text_groups(cx)[0].align, TextAlign::Right);
            assert_eq!(
                app.build_text_groups(cx)[0].direction,
                TextDirection::Vertical
            );
            assert_eq!(
                app.build_text_groups(cx)[0].text.text_params[0].align,
                TextAlign::Left
            );
        });
    }

    #[gpui_kit::test]
    fn custom_logo_attachment_keeps_identity_and_rejects_cycles(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (app, cx) =
            cx.add_window_view(|window, cx| AppView::new_with_settings_path(None, window, cx));
        app.update_in(cx, |app, window, cx| {
            app.add_text_group(window, cx);
            let target = app.text_groups[0].id;
            let child = app.text_groups[1].id;
            app.set_group_attachment(child, Some(target), Placement::Left, cx);
            assert!(!app.can_attach_group(target, child));
            app.set_group_attachment(target, Some(child), Placement::Right, cx);
            assert!(app.text_groups[0].attachment_target.is_none());
            let snapshot = app.current_watermark(cx);
            assert_eq!(
                snapshot.text_groups[1].attachment.as_ref().unwrap().target,
                0
            );
            app.restore_watermark(snapshot, window, cx);
            assert_eq!(
                app.text_groups[1].attachment_target,
                Some(app.text_groups[0].id)
            );
            app.backend
                .params_mut()
                .palette_card
                .set_attachment(Some(GroupAttachment {
                    target: 1,
                    side: Placement::Right,
                    gap: 0.01,
                }));
            app.remove_text_group(app.text_groups[0].id, cx);
            assert!(app.text_groups[0].attachment_target.is_none());
            assert_eq!(
                app.backend
                    .params()
                    .palette_card
                    .attachment()
                    .unwrap()
                    .target,
                0
            );
            app.remove_text_group(app.text_groups[0].id, cx);
            assert!(app.backend.params().palette_card.attachment().is_none());
        });
        cx.run_until_parked();
    }

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
