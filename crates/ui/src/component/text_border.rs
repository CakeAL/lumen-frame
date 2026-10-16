use gpui_kit::component::{
    Selectable as _, Sizable as _, button::Button, h_flex, switch::Switch, v_flex,
};
use gpui_kit::{App, Context, Entity, IntoElement, Subscription, Window, prelude::*};

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

pub(in crate::app) struct TextBorderControls {
    enabled: bool,
    sides: [bool; 4],
    width: NumberField,
    radius: NumberField,
    inset: NumberField,
    color: ColorField,
    _subscriptions: Vec<Subscription>,
}

impl TextBorderControls {
    pub(super) fn new(border: &TextBorder, window: &mut Window, cx: &mut Context<AppView>) -> Self {
        let width = NumberField::new(border.width(), 0.01, 2.0, 0.01, 2, 100.0, "%", window, cx);
        let radius = NumberField::new(border.radius(), 0.0, 5.0, 0.1, 1, 100.0, "%", window, cx);
        let inset = NumberField::new(border.inset(), 0.0, 5.0, 0.1, 1, 100.0, "%", window, cx);
        let color = ColorField::new(border.color(), window, cx);
        let mut subscriptions = width.subscribe(window, cx, |_, _| {});
        subscriptions.extend(radius.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(inset.subscribe(window, cx, |_, _| {}));
        subscriptions.extend(color.subscribe(window, cx, |_, _| {}));
        Self {
            enabled: border.enabled(),
            sides: std::array::from_fn(|ix| border.side(SIDES[ix].2)),
            width,
            radius,
            inset,
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
        border.set_color(self.color.value(cx));
        border
    }

    pub(super) fn render(&self, id: u64, view: Entity<AppView>, cx: &App) -> impl IntoElement {
        let enabled_view = view.clone();
        v_flex()
            .w_full()
            .gap_3()
            .child(
                Switch::new(("text-border-enabled", id))
                    .label("文字组边框")
                    .checked(self.enabled)
                    .on_change(move |enabled, _, cx| {
                        enabled_view.update(cx, |this, cx| {
                            if let Some(group) =
                                this.text_groups.iter_mut().find(|group| group.id == id)
                            {
                                group.border.enabled = *enabled;
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
                            Button::new((key, id))
                                .label(label)
                                .small()
                                .outline()
                                .flex_1()
                                .selected(self.sides[ix])
                                .tooltip(format!("{label}边框"))
                                .accessibility_label(format!("切换{label}边框"))
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        if let Some(group) =
                                            this.text_groups.iter_mut().find(|group| group.id == id)
                                        {
                                            group.border.sides[ix] = !group.border.sides[ix];
                                            this.refresh_preview(cx);
                                            cx.notify();
                                        }
                                    });
                                })
                        })),
                )
                .child(self.color.render("颜色", false, cx))
                .child(self.width.render("粗细", false, cx))
                .child(self.radius.render("圆角", false, cx))
                .child(self.inset.render("内边距", false, cx))
                .child(description(
                    "尺寸按照片高度计算；可单独启用任意一边作为分隔线",
                ))
            })
    }
}
