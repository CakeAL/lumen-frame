//! 原生模板富文本输入：普通文字连续排版，字段由输入引擎作为原子 token 管理。
use super::{
    super::AppView,
    field::{ColorField, rgb_to_hsla, warning},
    text_section::{FontSelect, INHERIT_FONT_LABEL, TextLine},
};
use crate::{
    media::ExifInfo,
    render::text::render_styled_template_preview,
    watermark::{
        TextParams, TextSpan,
        rich_text::{TextStyleChange, apply_style, rebase_spans, selection_range, style_at},
    },
};
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InlineToken, InputContent};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    h_flex,
    input::{Textarea, TextareaState},
    popover::Popover,
    searchable_list::SearchableVec,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, EntityInputHandler as _, Focusable as _, Font, FontStyle,
    FontWeight, IntoElement, RenderOnce, SharedString, StyledText, Subscription, TextRun,
    UnderlineStyle, WeakEntity, Window, div, prelude::*, px,
};
use std::{ops::Range, sync::LazyLock};

static TEMPLATE_FIELDS: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{[^{}\r\n]+\}").unwrap());

pub(super) struct RichTextEditor {
    pub spans: Vec<TextSpan>,
    source: String,
    selection: Range<usize>,
    editing_token: Option<usize>,
    tokens_dirty: bool,
    font: Entity<FontSelect>,
    color: ColorField,
    _subscriptions: Vec<Subscription>,
}

impl RichTextEditor {
    pub fn new(
        id: u64,
        template: &str,
        params: &TextParams,
        fonts: &[SharedString],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let mut items = vec![SharedString::from(INHERIT_FONT_LABEL)];
        for font in params
            .spans
            .iter()
            .filter_map(|span| span.style.font.as_ref())
            .chain(std::iter::once(&params.font))
        {
            if !font.is_empty() && !items.iter().any(|item| item.as_ref() == font) {
                items.push(font.clone().into());
            }
        }
        for font in fonts {
            if !items.contains(font) {
                items.push(font.clone());
            }
        }
        let configured: SharedString = if params.font.is_empty() {
            INHERIT_FONT_LABEL.into()
        } else {
            params.font.clone().into()
        };
        let index = items
            .iter()
            .position(|item| *item == configured)
            .unwrap_or(0);
        let font = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::new(items),
                vec![IndexPath::new(index)],
                window,
                cx,
            )
            .searchable(true)
        });
        let color = ColorField::new(params.color.unwrap_or([255, 255, 255]), window, cx);
        let mut subscriptions = color.subscribe_edit(window, cx, move |this, rgb, window, cx| {
            this.format_rich_text(id, TextStyleChange::Color(Some(rgb)), window, cx);
        });
        subscriptions.push(cx.subscribe_in(
            &font,
            window,
            move |this, state, event, window, cx| {
                if matches!(event, ComboboxEvent::Confirm(_)) {
                    let font = state
                        .read(cx)
                        .selected_values()
                        .first()
                        .map(ToString::to_string)
                        .filter(|font| font != INHERIT_FONT_LABEL);
                    this.format_rich_text(id, TextStyleChange::Font(font), window, cx);
                }
            },
        ));
        Self {
            spans: params.spans.clone(),
            source: template.to_owned(),
            selection: 0..0,
            editing_token: None,
            tokens_dirty: false,
            font,
            color,
            _subscriptions: subscriptions,
        }
    }
    pub fn text_changed(&mut self, text: &str) {
        self.tokens_dirty |= self.source != text;
        rebase_spans(&self.source, text, &mut self.spans);
        self.source = text.to_owned();
    }
}

pub(super) fn token_content(text: &str, editing_token: Option<usize>) -> InputContent {
    let mut content = InputContent::new(text.to_owned());
    for found in TEMPLATE_FIELDS.find_iter(text) {
        if editing_token == Some(found.start()) {
            continue;
        }
        let field = found.as_str();
        let token = InlineToken::new(format!("field:{field}"), field.to_owned())
            .with_label(field[1..field.len() - 1].to_owned());
        // Regex starts and ends outside braces, which are grapheme boundaries in ordinary templates.
        // Malformed pasted combining sequences remain editable plain text if the engine rejects them.
        if let Ok(next) = content.clone().with_token(found.range(), token) {
            content = next;
        }
    }
    content
}

fn text_runs(
    text: &str,
    params: &TextParams,
    default_font: &str,
    default_color: gpui_kit::Hsla,
) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    for (offset, ch) in text.char_indices() {
        let style = style_at(&params.spans, offset);
        let run = TextRun {
            len: ch.len_utf8(),
            font: Font {
                family: style
                    .font
                    .unwrap_or_else(|| params.resolved_font(default_font).to_owned())
                    .into(),
                weight: if style.bold.unwrap_or(params.bold) {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                },
                style: if style.italic.unwrap_or(params.italic) {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
                ..Default::default()
            },
            color: style
                .color
                .or(params.color)
                .map(rgb_to_hsla)
                .unwrap_or(default_color),
            underline: style
                .underline
                .unwrap_or(params.underline)
                .then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
            ..Default::default()
        };
        if let Some(last) = runs.last_mut() {
            let mut comparison = run.clone();
            comparison.len = last.len;
            if *last == comparison {
                last.len += run.len;
                continue;
            }
        }
        runs.push(run);
    }
    runs
}

fn sync_template_tokens(line: &mut TextLine, window: &mut Window, cx: &mut Context<AppView>) {
    if !line.rich.tokens_dirty {
        return;
    }
    let editing_token = line.rich.editing_token;
    let synced = line.template.update(cx, |state, cx| {
        // 组合输入尚未提交时，保留原生 IME 状态，下一次通知再识别字段。
        if state.marked_text_range(window, cx).is_some() {
            return false;
        }
        let text = state.value();
        let selected = state.selected_range();
        let content = token_content(&text, editing_token);
        for token in content.tokens() {
            if !state.tokens().iter().any(|existing| existing == token)
                && state
                    .replace_range_with_token(token.range(), token.token().clone(), window, cx)
                    .is_err()
            {
                return false;
            }
        }
        state.set_selected_range(selected, cx);
        true
    });
    line.rich.tokens_dirty = !synced;
}

/// 普通 Textarea 只显示模板与字段标签；最终字体和颜色统一在解析结果中预览。
#[derive(IntoElement)]
struct RichTextInput {
    state: Entity<TextareaState>,
    view: WeakEntity<AppView>,
    toolbar: AnyElement,
    id: u64,
}
impl RenderOnce for RichTextInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let view = self.view;
        let id = self.id;
        let textarea = Textarea::new(&self.state)
            .accessibility_id(format!("text-template-{id}"))
            .appearance(false)
            .bordered(false)
            .aria_label("富文本模板")
            .token(move |token, _, cx| {
                let label = token.token().label().clone();
                h_flex()
                    .id(("template-token", token.range().start))
                    .gap_1()
                    .px_1()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .border_1()
                    .border_color(if token.is_selected() {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .child(Icon::new(IconName::Braces).xsmall())
                    .child(label)
            })
            .on_token_click(move |event, window, cx| {
                view.update(cx, |this, cx| {
                    this.edit_rich_token(id, event.range(), window, cx);
                })
                .ok();
            });
        v_flex()
            .id(("rich-text-composer", id))
            .w_full()
            .min_w_0()
            .p_2()
            .gap_2()
            .rounded(cx.theme().radius)
            .bg(cx.theme().input)
            .border_1()
            .border_color(if focused {
                cx.theme().ring
            } else {
                cx.theme().border
            })
            .child(textarea)
            .child(self.toolbar)
    }
}

impl AppView {
    pub(super) fn rich_text_changed(&mut self, id: u64, text: &str, cx: &App) {
        if let Some(line) = self
            .text_groups
            .iter_mut()
            .flat_map(|group| &mut group.lines)
            .find(|line| line.id == id)
        {
            line.rich.text_changed(text);
            // 手输/粘贴字段时，光标仍在花括号内就继续普通编辑，避免首字后立即原子化。
            if line.rich.editing_token.is_none() {
                let selected = line.template.read(cx).selected_range();
                line.rich.editing_token = TEMPLATE_FIELDS
                    .find_iter(text)
                    .find(|field| selected.start > field.start() && selected.end < field.end())
                    .map(|field| field.start());
            }
        }
    }

    pub(super) fn sync_rich_toolbar(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(line) = self
            .text_groups
            .iter_mut()
            .flat_map(|group| &mut group.lines)
            .find(|line| line.id == id)
        else {
            return;
        };
        let input = line.template.read(cx);
        if !input.focus_handle(cx).is_focused(window) {
            return;
        }
        if input.value().as_ref() != line.rich.source {
            return;
        }
        let selected = input.selected_range();
        if let Some(start) = line.rich.editing_token {
            let end = line
                .rich
                .source
                .get(start..)
                .filter(|text| text.starts_with('{'))
                .and_then(|text| text.find('}').map(|offset| start + offset));
            if !end.is_some_and(|end| selected.start > start && selected.end <= end) {
                line.rich.editing_token = None;
                line.rich.tokens_dirty = true;
                cx.notify();
            }
        }
        sync_template_tokens(line, window, cx);
        let range = selection_range(&line.rich.source, line.template.read(cx).selected_range());
        if line.rich.selection == range {
            return;
        }
        line.rich.selection = range.clone();
        let style = if range.is_empty() {
            Default::default()
        } else {
            style_at(&line.rich.spans, range.start)
        };
        let font: SharedString = style
            .font
            .or_else(|| (!line.to_params(cx).font.is_empty()).then(|| line.to_params(cx).font))
            .unwrap_or_else(|| INHERIT_FONT_LABEL.into())
            .into();
        line.rich.font.update(cx, |state, cx| {
            state.set_selected_values(&[font], window, cx)
        });
        line.rich.color.sync(
            style
                .color
                .or_else(|| (!line.auto_color).then(|| line.color.value(cx)))
                .unwrap_or([255, 255, 255]),
            window,
            cx,
        );
        cx.notify();
    }

    fn format_rich_text(
        &mut self,
        id: u64,
        change: TextStyleChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(line) = self
            .text_groups
            .iter_mut()
            .flat_map(|group| &mut group.lines)
            .find(|line| line.id == id)
        else {
            return;
        };
        let text = line.template.read(cx).value().to_string();
        if line.rich.source != text {
            line.rich.text_changed(&text);
        }
        let selected = selection_range(&text, line.template.read(cx).selected_range());
        let range = if selected.is_empty() {
            0..text.len()
        } else {
            selected.clone()
        };
        let mut span_change = change.clone();
        if selected.is_empty() {
            match change {
                TextStyleChange::Font(font) => {
                    let value: SharedString =
                        font.unwrap_or_else(|| INHERIT_FONT_LABEL.into()).into();
                    line.font.update(cx, |state, cx| {
                        state.set_selected_values(&[value], window, cx)
                    });
                    span_change = TextStyleChange::Font(None);
                }
                TextStyleChange::Color(color) => {
                    line.auto_color = color.is_none();
                    if let Some(rgb) = color {
                        line.color.sync(rgb, window, cx);
                    }
                    span_change = TextStyleChange::Color(None);
                }
                TextStyleChange::Bold(value) => line.bold = value,
                TextStyleChange::Italic(value) => line.italic = value,
                TextStyleChange::Underline(value) => line.underline = value,
                TextStyleChange::Reset => {
                    line.bold = false;
                    line.italic = false;
                    line.underline = false;
                    line.auto_color = true;
                    line.font.update(cx, |state, cx| {
                        state.set_selected_values(&[INHERIT_FONT_LABEL.into()], window, cx)
                    });
                }
            }
        }
        apply_style(&text, &mut line.rich.spans, range, span_change);
        self.refresh_preview(cx);
        cx.notify();
    }

    fn edit_rich_token(
        &mut self,
        id: u64,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(line) = self
            .text_groups
            .iter_mut()
            .flat_map(|group| &mut group.lines)
            .find(|line| line.id == id)
        else {
            return;
        };
        line.rich.editing_token = Some(range.start);
        line.template.update(cx, |state, cx| {
            let text = state.value();
            state.set_selected_range(range.clone(), cx);
            state.replace(text[range.clone()].to_owned(), window, cx);
            state.set_selected_range(range.start + 1..range.end - 1, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn finish_rich_token_edit(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(line) = self
            .text_groups
            .iter_mut()
            .flat_map(|group| &mut group.lines)
            .find(|line| line.id == id)
        else {
            return;
        };
        if line.rich.editing_token.take().is_some() {
            line.rich.tokens_dirty = true;
            sync_template_tokens(line, window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_rich_composer(
        &self,
        line: &TextLine,
        view: Entity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        RichTextInput {
            state: line.template.clone(),
            view: view.downgrade(),
            toolbar: self
                .render_rich_text_controls(line, view, cx)
                .into_any_element(),
            id: line.id,
        }
    }

    fn render_rich_text_controls(
        &self,
        line: &TextLine,
        view: Entity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let id = line.id;
        let range = line.template.read(cx).selected_range();
        let style = if range.is_empty() {
            Default::default()
        } else {
            style_at(&line.rich.spans, range.start)
        };
        let color = line.rich.color.clone();
        let color_view = view.clone();
        let inherit_label = if range.is_empty() {
            "自动颜色"
        } else {
            "跟随行颜色"
        };
        let mut toolbar = h_flex()
            .w_full()
            .gap_1()
            .child(
                div().w_7().h_7().flex_shrink_0().child(
                    Combobox::new(&line.rich.font)
                        .appearance(false)
                        .small()
                        .p_0()
                        .border_0()
                        .search_placeholder("搜索字体…")
                        .menu_width(gpui_kit::rems(18.))
                        .render_trigger(move |trigger, _, _| {
                            Button::new(("rich-text-font", id))
                                .icon(IconName::Type)
                                .accessibility_label("字体")
                                .tooltip("字体")
                                .small()
                                .ghost()
                                .selected(trigger.is_open())
                        }),
                ),
            )
            .child(
                Popover::new(("rich-text-color-popup", id))
                    .trigger(
                        Button::new(("rich-text-color", id))
                            .icon(IconName::Baseline)
                            .accessibility_label("文字颜色")
                            .tooltip("文字颜色")
                            .small()
                            .ghost(),
                    )
                    .content(move |_, _, _| {
                        let view = color_view.clone();
                        v_flex().gap_2().child(color.render_control()).child(
                            Button::new(("rich-text-inherit-color", id))
                                .label(inherit_label)
                                .small()
                                .ghost()
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.format_rich_text(
                                            id,
                                            TextStyleChange::Color(None),
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        )
                    }),
            );
        for (name, label, icon, selected, change) in [
            (
                "rich-text-bold",
                "加粗",
                IconName::Bold,
                style.bold.unwrap_or(line.bold),
                TextStyleChange::Bold(!style.bold.unwrap_or(line.bold)),
            ),
            (
                "rich-text-italic",
                "斜体",
                IconName::Italic,
                style.italic.unwrap_or(line.italic),
                TextStyleChange::Italic(!style.italic.unwrap_or(line.italic)),
            ),
            (
                "rich-text-underline",
                "下划线",
                IconName::Underline,
                style.underline.unwrap_or(line.underline),
                TextStyleChange::Underline(!style.underline.unwrap_or(line.underline)),
            ),
        ] {
            let view = view.clone();
            toolbar = toolbar.child(
                Button::new((name, id))
                    .icon(icon)
                    .accessibility_label(label)
                    .tooltip(label)
                    .small()
                    .ghost()
                    .selected(selected)
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.format_rich_text(id, change.clone(), window, cx)
                        });
                    }),
            );
        }
        toolbar
            .child(self.render_template_field_menu(line.template.clone(), view.clone(), id))
            .child(div().flex_1())
            .child(
                Button::new(("rich-text-reset", id))
                    .icon(IconName::RemoveFormatting)
                    .accessibility_label("清除格式")
                    .tooltip("清除格式")
                    .small()
                    .ghost()
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            this.format_rich_text(id, TextStyleChange::Reset, window, cx)
                        });
                    }),
            )
    }

    pub(super) fn render_rich_text_result(
        &self,
        line: &TextLine,
        exif: Option<&ExifInfo>,
        time_format: &str,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let Some(exif) = exif else {
            return warning("选中的照片没有 EXIF 信息，这一行不会渲染。", cx);
        };
        let mut params = line.to_params(cx);
        let (text, spans) = render_styled_template_preview(
            &line.template.read(cx).value(),
            &params,
            exif,
            time_format,
            &self.params.custom_text,
        );
        if text.is_empty() {
            return warning("当前模板解析结果为空，这一行不会渲染。", cx);
        }
        params.spans = spans;
        let runs = text_runs(
            &text,
            &params,
            &self.params.default_font,
            cx.theme().foreground,
        );
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("解析结果"),
            )
            .child(StyledText::new(text).with_runs(runs))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Render, TestAppContext};
    use std::{cell::RefCell, rc::Rc};
    struct Harness {
        app: Entity<AppView>,
        _observe: Subscription,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let app = self.app.read(cx);
            let line = &app.text_groups[0].lines[0];
            v_flex()
                .p_4()
                .gap_2()
                .child(app.render_rich_composer(line, self.app.clone(), cx))
        }
    }
    #[gpui_kit::test]
    fn direct_rich_editor_preserves_selection_and_atomic_token_history(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let slot = slot.clone();
            move |window, cx| {
                let app = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                app.update(cx, |app, cx| {
                    app.text_groups[0].lines[0]
                        .template
                        .update(cx, |input, cx| {
                            input.set_value(token_content("AB{ISO}中文", None), window, cx)
                        });
                    let id = app.text_groups[0].lines[0].id;
                    app.rich_text_changed(id, "AB{ISO}中文", cx);
                });
                slot.borrow_mut().replace(app.clone());
                let harness = cx.new(|cx| {
                    let observe = cx.observe(&app, |_, _, cx| cx.notify());
                    Harness {
                        app,
                        _observe: observe,
                    }
                });
                Root::new(harness, window, cx)
            }
        });
        let app = slot.borrow().clone().unwrap();
        cx.run_until_parked();
        let id = app.read_with(cx, |app, _| app.text_groups[0].lines[0].id);
        cx.update(|window, cx| {
            window.click(("rich-text-italic", id), cx);
            app.update(cx, |app, cx| {
                app.text_groups[0].lines[0]
                    .template
                    .update(cx, |input, cx| input.focus(window, cx));
            });
            window.press("cmd-left", cx);
            window.press("right", cx);
            window.press("right", cx);
            window.press("right", cx);
            window.press("backspace", cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            let line = &app.text_groups[0].lines[0];
            assert!(line.italic, "空选区设置整行");
            assert_eq!(line.template.read(cx).value().as_ref(), "AB中文");
        });
        cx.update(|window, cx| {
            window.press("cmd-z", cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            let input = app.text_groups[0].lines[0].template.read(cx);
            assert_eq!(input.value().as_ref(), "AB{ISO}中文");
            assert_eq!(input.tokens().len(), 1);
            assert_eq!(input.tokens()[0].token().text().as_ref(), "{ISO}");
        });
        cx.update(|window, cx| {
            window.press("cmd-left", cx);
            window.press("shift-right", cx);
            window.click(("rich-text-underline", id), cx);
            window.click(("rich-text-color", id), cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.click("gradient-color-hex", cx);
            window.press("cmd-a", cx);
            window.input("#FF0000", cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let line = &app.text_groups[0].lines[0];
            assert_eq!(style_at(&line.rich.spans, 0).color, Some([255, 0, 0]));
            assert_eq!(style_at(&line.rich.spans, 0).underline, Some(true));
            assert_eq!(style_at(&line.rich.spans, 1).color, None);
            assert!(!line.underline, "选区只设置局部下划线");
        });
        cx.update(|window, cx| {
            window.press("escape", cx);
        });
        cx.run_until_parked();
        let bounds = app.read_with(cx, |app, cx| {
            app.text_groups[0].lines[0]
                .template
                .read(cx)
                .range_to_bounds(&(2..7))
                .unwrap()
        });
        cx.simulate_click(bounds.center(), Default::default());
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            let input = app.text_groups[0].lines[0].template.read(cx);
            assert!(input.tokens().is_empty(), "点击字段后展开为可编辑文字");
            assert_eq!(input.selected_range(), 3..6);
        });
        cx.update(|window, cx| {
            window.input("型号", cx);
            window.press("cmd-right", cx);
        });
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            let input = app.text_groups[0].lines[0].template.read(cx);
            assert_eq!(input.value().as_ref(), "AB{型号}中文");
            assert_eq!(input.tokens()[0].token().text().as_ref(), "{型号}");
        });
    }
}
