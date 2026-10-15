//! 背景模式、可增删的渐变色标和照片色卡。

use super::{
    super::{AppView, behavior::palette::PaletteStatus},
    field::{description, field, rgb_to_hsla, warning},
};
use crate::watermark::{BackgroundKind, PaletteTheme};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    select::Select,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{Context, IntoElement, black, div, prelude::*, white};

pub(in crate::ui::app) const PALETTE_THEMES: &[(&str, PaletteTheme)] = &[
    ("均衡（默认）", PaletteTheme::Default),
    ("占比优先（Basic）", PaletteTheme::Basic),
    ("多彩（Colorful）", PaletteTheme::Colorful),
    ("鲜艳（Vivid）", PaletteTheme::Vivid),
    ("柔和（Muted）", PaletteTheme::Muted),
    ("浅色（Light）", PaletteTheme::Light),
    ("深色（Dark）", PaletteTheme::Dark),
];

const DIRECTIONS: &[(&str, &str, f64)] = &[
    ("↑", "向上", 0.0),
    ("↗", "向右上", 45.0),
    ("→", "向右", 90.0),
    ("↘", "向右下", 135.0),
    ("↓", "向下", 180.0),
    ("↙", "向左下", 225.0),
    ("←", "向左", 270.0),
    ("↖", "向左上", 315.0),
];

impl AppView {
    pub(in crate::ui::app) fn render_background_section(
        &self,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let kind = self.params.background_kind();
        GroupBox::new()
            .id("background-section")
            .title("背景")
            .child(
                TabBar::new("background-mode")
                    .segmented()
                    .w_full()
                    .children([
                        Tab::new().label("照片模糊").flex_1(),
                        Tab::new().label("纯色").flex_1(),
                        Tab::new().label("渐变").flex_1(),
                    ])
                    .selected_index(match kind {
                        BackgroundKind::Photo => 0,
                        BackgroundKind::Solid => 1,
                        BackgroundKind::Gradient => 2,
                    })
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.params.set_background_kind(match index {
                            1 => BackgroundKind::Solid,
                            2 => BackgroundKind::Gradient,
                            _ => BackgroundKind::Photo,
                        });
                        this.schedule_palette_for_selected(cx);
                        this.refresh_preview(cx);
                        cx.notify();
                    })),
            )
            .when(kind == BackgroundKind::Photo, |this| {
                this.child(self.controls.blur_sigma.render("模糊强度", false, cx))
            })
            .when(kind == BackgroundKind::Solid, |this| {
                this.child(self.controls.background.render("背景颜色", false, cx))
            })
            .when(kind == BackgroundKind::Gradient, |this| {
                this.child(self.render_gradient_controls(cx))
            })
    }

    fn render_gradient_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let sources = self
            .params
            .gradient_palette
            .slots(self.params.gradient.colors().len());
        v_flex()
            .w_full()
            .gap_3()
            .child(
                div()
                    .id("gradient-angle")
                    .child(self.controls.gradient_angle.render("角度", false, cx)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .children(DIRECTIONS.iter().map(|&(arrow, name, angle)| {
                        Button::new(("gradient-direction", angle as usize))
                            .label(arrow)
                            .small()
                            .w_6()
                            .p_0()
                            .outline()
                            .selected((self.params.gradient.angle() - angle).abs() < 0.01)
                            .tooltip(format!("{name} · {angle}°"))
                            .accessibility_label(format!("渐变{name} {angle}°"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_gradient_angle(angle, window, cx)
                            }))
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(div().text_sm().child("渐变颜色"))
                    .child(
                        Button::new("gradient-color-add")
                            .label("添加颜色")
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_gradient_color(window, cx)
                            })),
                    ),
            )
            .children(
                self.background_state
                    .colors
                    .iter()
                    .enumerate()
                    .map(|(index, control)| {
                        let id = control.id;
                        v_flex()
                            .id(("gradient-stop", id))
                            .w_full()
                            .gap_1()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .justify_between()
                                    .child(div().text_sm().child(format!("颜色 {}", index + 1)))
                                    .child(control.color.render_control())
                                    .child(
                                        Button::new(("gradient-color-remove", id))
                                            .label("×")
                                            .small()
                                            .ghost()
                                            .tooltip("删除这个颜色")
                                            .accessibility_label(format!(
                                                "删除渐变颜色 {}",
                                                index + 1
                                            ))
                                            .disabled(self.background_state.colors.len() <= 2)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.remove_gradient_color(id, window, cx)
                                            })),
                                    ),
                            )
                            .when_some(sources[index], |this, rank| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("照片色卡 · 第 {} 色", rank + 1)),
                                )
                            })
                    }),
            )
            .child(self.render_photo_palette(cx))
    }

    fn render_photo_palette(&self, cx: &Context<Self>) -> impl IntoElement {
        let status = self.selected_palette();
        let loading = matches!(status, Some(PaletteStatus::Loading));
        v_flex()
            .w_full()
            .gap_3()
            .child(field(
                "照片色卡风格",
                Select::new(&self.controls.palette_theme)
                    .id("photo-palette-theme")
                    .w_full(),
                cx,
            ))
            .child(
                Button::new("photo-palette-extract")
                    .label(if loading {
                        "正在取色…"
                    } else {
                        "提取照片色卡"
                    })
                    .outline()
                    .small()
                    .disabled(self.selected_photo_id().is_none() || loading)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.extract_photo_palette(window, cx)),
                    ),
            )
            .when(self.selected_photo_id().is_none(), |this| {
                this.child(description("添加照片后可提取色卡"))
            })
            .when_some(
                if let Some(PaletteStatus::Failed(error)) = status {
                    Some(error.clone())
                } else {
                    None
                },
                |this, error| this.child(warning(error, cx)),
            )
            .when_some(
                if let Some(PaletteStatus::Ready(swatches)) = status {
                    Some(swatches)
                } else {
                    None
                },
                |this, swatches| {
                    this.child(h_flex().gap_1().children(swatches.iter().enumerate().map(
                        |(rank, swatch)| {
                            let rgb = swatch.rgb();
                            let [r, g, b] = rgb;
                            let selected = self.background_state.selected.contains(&rank);
                            let ink = if u32::from(r) + u32::from(g) + u32::from(b) > 384 {
                                black()
                            } else {
                                white()
                            };
                            Button::new(("photo-palette-color", rank))
                                .size_8()
                                .p_0()
                                .outline()
                                .selected(selected)
                                .child(
                                    div()
                                        .size_5()
                                        .rounded(cx.theme().radius)
                                        .bg(rgb_to_hsla(rgb))
                                        .text_color(ink)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(selected, |this| this.child("✓")),
                                )
                                .tooltip(format!(
                                    "第 {} 色 · #{r:02X}{g:02X}{b:02X} · 占比 {:.1}%",
                                    rank + 1,
                                    swatch.ratio() * 100.0
                                ))
                                .accessibility_label(format!(
                                    "选择色卡第 {} 色 #{r:02X}{g:02X}{b:02X}",
                                    rank + 1
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.background_state.selected.remove(&rank) {
                                        this.background_state.selected.insert(rank);
                                    }
                                    cx.notify();
                                }))
                        },
                    )))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                Button::new("photo-palette-add-selected")
                                    .label(format!(
                                        "添加所选（{}）",
                                        self.background_state.selected.len()
                                    ))
                                    .small()
                                    .outline()
                                    .disabled(self.background_state.selected.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.append_palette_colors(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("photo-palette-add-all")
                                    .label("全部添加")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.append_palette_colors(true, window, cx)
                                    })),
                            ),
                    )
                    .child(description(
                        "色卡颜色随照片匹配；应用到其它照片时，使用那张照片的对应代表色",
                    ))
                },
            )
    }
}
