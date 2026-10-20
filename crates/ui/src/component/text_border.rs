use gpui_kit::component::{
    Selectable as _, Sizable as _, button::Button, h_flex, switch::Switch, v_flex,
};
use gpui_kit::{App, Context, ElementId, Entity, IntoElement, Subscription, Window, prelude::*};

use super::{
    super::AppView,
    field::{ColorField, NumberField, description},
};
use crate::backend::watermark::{Placement, TextBorder};

const SIDES: &[(&str, &str, Placement)] = &[
    ("text-border-top", "上", Placement::Up),
    ("text-border-bottom", "下", Placement::Bottom),
    ("text-border-left", "左", Placement::Left),
    ("text-border-right", "右", Placement::Right),
];

#[derive(Clone, Copy)]
enum BorderTarget {
    Group(u64),
    Line(u64),
}

impl BorderTarget {
    fn element_id(self, key: &'static str) -> ElementId {
        match self {
            Self::Group(id) => (key, id).into(),
            Self::Line(id) => (ElementId::from(("text-line-border", id)), key).into(),
        }
    }

    fn controls(self, view: &mut AppView) -> Option<&mut TextBorderControls> {
        match self {
            Self::Group(id) => view
                .text_groups
                .iter_mut()
                .find(|group| group.id == id)
                .map(|group| &mut group.border),
            Self::Line(id) => view
                .text_groups
                .iter_mut()
                .flat_map(|group| &mut group.lines)
                .find(|line| line.id == id)
                .map(|line| &mut line.border),
        }
    }
}

pub(in crate::app) struct TextBorderControls {
    /// 是否有显式配置，避免滑块的 f32 投影误把默认行边框写成覆盖项。
    configured: bool,
    enabled: bool,
    sides: [bool; 4],
    width: NumberField,
    radius: NumberField,
    inset: NumberField,
    min_width: NumberField,
    min_height: NumberField,
    color: ColorField,
    _subscriptions: Vec<Subscription>,
}

impl TextBorderControls {
    pub(super) fn new(border: &TextBorder, window: &mut Window, cx: &mut Context<AppView>) -> Self {
        let width = NumberField::new(border.width(), 0.01, 2.0, 0.01, 2, 100.0, "%", window, cx);
        let radius = NumberField::new(border.radius(), 0.0, 5.0, 0.1, 1, 100.0, "%", window, cx);
        let inset = NumberField::new(border.inset(), 0.0, 5.0, 0.1, 1, 100.0, "%", window, cx);
        let min_width = NumberField::new(
            border.min_width(),
            0.0,
            100.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let min_height = NumberField::new(
            border.min_height(),
            0.0,
            100.0,
            0.1,
            1,
            100.0,
            "%",
            window,
            cx,
        );
        let color = ColorField::new(border.color(), window, cx);
        let mut subscriptions = width.subscribe(window, cx, |_, _| {});
        subscriptions.extend(radius.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(inset.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(min_width.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(min_height.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(color.subscribe(window, cx, |_, _| {}));
        Self {
            configured: border != &TextBorder::default(),
            enabled: border.enabled(),
            sides: std::array::from_fn(|ix| border.side(SIDES[ix].2)),
            width,
            radius,
            inset,
            min_width,
            min_height,
            color,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn to_border(&self, cx: &App) -> TextBorder {
        let mut border = TextBorder::default();
        border.set_enabled(self.enabled);
        for ((_, _, side), enabled) in SIDES.iter().zip(self.sides) {
            border.set_side(*side, enabled);
        }
        border.set_width(self.width.value(cx));
        border.set_radius(self.radius.value(cx));
        border.set_inset(self.inset.value(cx));
        border.set_min_width(self.min_width.value(cx));
        border.set_min_height(self.min_height.value(cx));
        border.set_color(self.color.value(cx));
        border
    }

    pub(super) fn to_optional_border(&self, cx: &App) -> Option<TextBorder> {
        self.configured.then(|| self.to_border(cx))
    }

    pub(super) fn render(
        &self,
        id: u64,
        background_enabled: bool,
        view: Entity<AppView>,
        cx: &App,
    ) -> impl IntoElement {
        self.render_target(BorderTarget::Group(id), background_enabled, view, cx)
    }

    pub(super) fn render_line(&self, id: u64, view: Entity<AppView>, cx: &App) -> impl IntoElement {
        self.render_target(BorderTarget::Line(id), false, view, cx)
    }

    fn render_target(
        &self,
        target: BorderTarget,
        background_enabled: bool,
        view: Entity<AppView>,
        cx: &App,
    ) -> impl IntoElement {
        let enabled_view = view.clone();
        v_flex()
            .w_full()
            .gap_3()
            .child(
                Switch::new(target.element_id("text-border-enabled"))
                    .label(match target { BorderTarget::Group(_) => "文字组边框", BorderTarget::Line(_) => "本行边框" })
                    .checked(self.enabled)
                    .on_change(move |enabled, _, cx| {
                        enabled_view.update(cx, |this, cx| {
                            if let Some(border) = target.controls(this) {
                                border.configured = true;
                                border.enabled = *enabled;
                                this.refresh_preview(cx);
                                cx.notify();
                            }
                        });
                    }),
            )
            .when(self.enabled, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .children(SIDES.iter().enumerate().map(|(ix, &(key, label, _))| {
                            let view = view.clone();
                            Button::new(target.element_id(key))
                                .label(label)
                                .small()
                                .outline()
                                .flex_1()
                                .selected(self.sides[ix])
                                .tooltip(format!("{label}边框"))
                                .accessibility_label(format!("切换{label}边框"))
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        if let Some(border) = target.controls(this) {
                                            border.sides[ix] = !border.sides[ix];
                                            this.refresh_preview(cx);
                                            cx.notify();
                                        }
                                    });
                                })
                        })),
                )
                .child(self.color.render("颜色", false, cx))
                .child(self.width.render("粗细", false, cx))
            })
            .when(self.enabled || background_enabled, |this| {
                this.child(self.radius.render("圆角", false, cx))
                .child(self.inset.render("内边距", false, cx))
                .child(self.min_width.render("最小宽度", false, cx))
                .child(self.min_height.render("最小高度", false, cx))
                .when(matches!(target, BorderTarget::Line(_)), |this| {
                    this.child(description("只包围本行内容，行距留在框外；单位或说明可放在另一行。"))
                })
                .child(description(
                    match target {
                        BorderTarget::Group(_) => "尺寸按照片高度计算；背景与边框共用圆角、内边距和最小尺寸。宽高为 0 时随内容，文字在框内居中。",
                        BorderTarget::Line(_) => "尺寸按照片高度计算；宽高为 0 时随内容。设置相同宽高可统一多个框的大小，文字在框内居中。",
                    },
                ))
            })
    }
}
