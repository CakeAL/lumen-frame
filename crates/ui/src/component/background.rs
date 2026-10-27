//! 背景模式、可增删的渐变色标和照片色卡。

use super::{
    super::{
        AppView,
        behavior::palette::{GradientStopControl, PaletteStatus},
    },
    field::{description, field, parameter_card, rgb_to_hsla, warning},
};
use crate::backend::watermark::{BackgroundKind, PaletteTheme};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    popover::Popover,
    select::Select,
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    Context, EntityId, IntoElement, KeyDownEvent, Render, Window, black, div, prelude::*, white,
};

pub(in crate::app) const PALETTE_THEMES: &[(&str, PaletteTheme)] = &[
    ("均衡（默认）", PaletteTheme::Default),
    ("占比优先（Basic）", PaletteTheme::Basic),
    ("多彩（Colorful）", PaletteTheme::Colorful),
    ("鲜艳（Vivid）", PaletteTheme::Vivid),
    ("柔和（Muted）", PaletteTheme::Muted),
    ("浅色（Light）", PaletteTheme::Light),
    ("深色（Dark）", PaletteTheme::Dark),
];

// 按屏幕空间排列，中心留空。
const DIRECTIONS: &[(&str, &str, f64)] = &[
    ("↖", "向左上", 315.0),
    ("↑", "向上", 0.0),
    ("↗", "向右上", 45.0),
    ("←", "向左", 270.0),
    ("", "", -1.0),
    ("→", "向右", 90.0),
    ("↙", "向左下", 225.0),
    ("↓", "向下", 180.0),
    ("↘", "向右下", 135.0),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum PaletteTarget {
    Background,
    Border,
}

#[derive(Clone)]
struct DraggedGradientColor {
    owner: EntityId,
    id: u64,
    rgb: [u8; 3],
}

impl Render for DraggedGradientColor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_8()
            .rounded(cx.theme().radius)
            .border_2()
            .border_color(cx.theme().primary)
            .bg(rgb_to_hsla(self.rgb))
            .shadow_sm()
    }
}

impl AppView {
    pub(in crate::app) fn render_background_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let kind = self.backend.params().background_kind();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(parameter_card(
                "inspector-background-style-card",
                v_flex()
                    .w_full()
                    .gap_3()
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
                                this.backend.params_mut().set_background_kind(match index {
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
                            .child(self.render_photo_palette(PaletteTarget::Background, cx))
                    })
                    .when(kind == BackgroundKind::Gradient, |this| {
                        this.child(self.render_gradient_controls(cx))
                    }),
            ))
            .child(parameter_card(
                "inspector-background-grain-card",
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        Switch::new("background-grain")
                            .label("颗粒噪声")
                            .checked(self.backend.params().background_grain)
                            .on_change(cx.listener(|this, enabled: &bool, _, cx| {
                                this.backend.params_mut().background_grain = *enabled;
                                this.refresh_preview(cx);
                                cx.notify();
                            })),
                    )
                    .when(self.backend.params().background_grain, |this| {
                        this.child(self.controls.grain_strength.render("颗粒强度", false, cx))
                            .child(description("仅作用于背景，照片与文字保持清晰。"))
                    }),
            ))
    }

    pub(in crate::app) fn render_photo_border_section(
        &self,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let enabled = self.backend.params().photo_border.enabled();
        v_flex()
            .w_full()
            .gap_3()
            .child(
                Switch::new("photo-border-enabled")
                    .label("照片描边")
                    .checked(enabled)
                    .on_change(cx.listener(|this, enabled: &bool, _, cx| {
                        this.backend.params_mut().photo_border.set_enabled(*enabled);
                        this.schedule_palette_for_selected(cx);
                        this.refresh_preview(cx);
                        cx.notify();
                    })),
            )
            .when(enabled, |this| {
                this.child(self.controls.photo_border_width.render("宽度", false, cx))
                    .child(description("按照片高度的 0.1%–5% 向外扩展，圆角跟随照片"))
                    .child(
                        self.controls
                            .photo_border_color
                            .render("描边颜色", false, cx),
                    )
                    .child(self.render_photo_palette(PaletteTarget::Border, cx))
            })
    }

    fn render_gradient_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let colors = self.gradient_display_colors();
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .id("gradient-angle")
                    .w_full()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(div().text_sm().child("角度"))
                            .child(self.controls.gradient_angle.render_input()),
                    )
                    .child(v_flex().id("gradient-directions").gap_1().children(
                        DIRECTIONS.chunks(3).map(|row| {
                            h_flex()
                                .gap_1()
                                .children(row.iter().map(|&(arrow, name, angle)| {
                                    if angle < 0.0 {
                                        return Button::new("gradient-direction-center")
                                            .small()
                                            .size_6()
                                            .p_0()
                                            .outline()
                                            .disabled(true)
                                            .into_any_element();
                                    }
                                    Button::new(("gradient-direction", angle as usize))
                                        .label(arrow)
                                        .small()
                                        .size_6()
                                        .p_0()
                                        .outline()
                                        .selected(
                                            (self.backend.params().gradient.angle() - angle).abs()
                                                < 0.01,
                                        )
                                        .tooltip(format!("{name} · {angle}°"))
                                        .accessibility_label(format!("渐变{name} {angle}°"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.set_gradient_angle(angle, window, cx)
                                        }))
                                        .into_any_element()
                                }))
                        }),
                    )),
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
            .child(
                h_flex()
                    .id("gradient-colors")
                    .w_full()
                    .flex_wrap()
                    .gap_1()
                    .children(
                        self.background_state
                            .colors
                            .iter()
                            .zip(colors)
                            .map(|(control, rgb)| self.render_gradient_color(control, rgb, cx)),
                    ),
            )
            .child(self.render_photo_palette(PaletteTarget::Background, cx))
    }

    fn render_gradient_color(
        &self,
        control: &GradientStopControl,
        rgb: [u8; 3],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let id = control.id;
        let color = control.color.clone();
        let owner = cx.entity_id();
        let view = cx.entity().downgrade();
        let drag_view = view.clone();
        let delete_view = view.clone();
        let can_remove = self.background_state.colors.len() > 1;
        div()
            .id(("gradient-stop", id))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && !cx.has_active_drag() && this.background_state.editor != Some(id) {
                    this.background_state.editor = Some(id);
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.alt
                    && matches!(event.keystroke.key.as_str(), "left" | "right")
                {
                    let position = this
                        .background_state
                        .colors
                        .iter()
                        .position(|control| control.id == id);
                    if let Some(position) = position {
                        let adjacent = if event.keystroke.key == "left" {
                            position.checked_sub(1)
                        } else {
                            Some(position + 1)
                        };
                        if let Some(target) = adjacent
                            .and_then(|i| this.background_state.colors.get(i))
                            .map(|control| control.id)
                        {
                            this.swap_gradient_colors(id, target, window, cx);
                        }
                    }
                    cx.stop_propagation();
                }
            }))
            .child(
                Popover::new(("gradient-color-editor", id))
                    // 内层取色器打开时，点击它的色块必须先提交颜色，保留外层编辑器。
                    .overlay_closable(!color.is_open(cx))
                    .open(self.background_state.editor == Some(id))
                    .on_open_change(move |open, _, cx| {
                        let _ = view.update(cx, |this, cx| {
                            if *open {
                                this.background_state.editor = Some(id)
                            } else if this.background_state.editor == Some(id) {
                                this.background_state.editor = None
                            }
                            cx.notify();
                        });
                    })
                    .trigger(
                        Button::new(("gradient-color-swatch", id))
                            .size_8()
                            .p_0()
                            .outline()
                            .map(|mut button| {
                                // Button 已有稳定身份，用 GPUI 的命令式拖动 API 覆盖整个命中区。
                                button.interactivity().on_drag(
                                    DraggedGradientColor { owner, id, rgb },
                                    move |dragged, _, _, cx| {
                                        let _ = drag_view.update(cx, |this, cx| {
                                            this.background_state.editor = None;
                                            cx.notify();
                                        });
                                        cx.new(|_| dragged.clone())
                                    },
                                );
                                button
                            })
                            .can_drop(move |value, _, _| {
                                value.downcast_ref::<DraggedGradientColor>().is_some_and(
                                    |dragged| dragged.owner == owner && dragged.id != id,
                                )
                            })
                            .drag_over::<DraggedGradientColor>(move |style, dragged, _, cx| {
                                if dragged.owner == owner && dragged.id != id {
                                    style.border_2().border_color(cx.theme().primary)
                                } else {
                                    style
                                }
                            })
                            .on_drop(cx.listener(
                                move |this, dragged: &DraggedGradientColor, window, cx| {
                                    if dragged.owner == cx.entity_id() {
                                        cx.stop_propagation();
                                        this.swap_gradient_colors(dragged.id, id, window, cx);
                                    }
                                },
                            ))
                            .accessibility_label(format!(
                                "编辑渐变颜色 #{:02X}{:02X}{:02X}",
                                rgb[0], rgb[1], rgb[2]
                            ))
                            .child(
                                div()
                                    .size_5()
                                    .rounded(cx.theme().radius)
                                    .bg(rgb_to_hsla(rgb)),
                            ),
                    )
                    .content(move |_, _, _| {
                        let view = delete_view.clone();
                        h_flex()
                            .id(("gradient-color-actions", id))
                            .gap_2()
                            .child(color.render_control())
                            .child(
                                Button::new(("gradient-color-remove", id))
                                    .label("删除")
                                    .small()
                                    .ghost()
                                    .disabled(!can_remove)
                                    .on_click(move |_, window, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            this.background_state.editor = None;
                                            this.remove_gradient_color(id, window, cx);
                                        });
                                    }),
                            )
                    }),
            )
    }

    fn render_photo_palette(&self, target: PaletteTarget, cx: &Context<Self>) -> impl IntoElement {
        let border = target == PaletteTarget::Border;
        let prefix = if border {
            "border-palette-color"
        } else {
            "photo-palette-color"
        };
        let theme_control = if border {
            &self.controls.border_palette_theme
        } else {
            &self.controls.palette_theme
        };
        let status = self.selected_palette();
        let loading = matches!(status, Some(PaletteStatus::Loading));
        let solid = self.backend.params().background_kind() == BackgroundKind::Solid;
        v_flex()
            .w_full()
            .gap_3()
            .child(field(
                "照片色卡风格",
                Select::new(theme_control)
                    .id(if border {
                        "border-palette-theme"
                    } else {
                        "photo-palette-theme"
                    })
                    .w_full(),
                cx,
            ))
            .when(loading, |this| this.child(description("正在提取照片色卡…")))
            .when(self.selected_photo_id().is_none(), |this| {
                this.child(description("添加照片后可提取色卡"))
            })
            .when_some(
                if let Some(PaletteStatus::Failed(error)) = status {
                    Some(error.clone())
                } else {
                    None
                },
                |this, error| {
                    this.child(warning(error, cx)).child(
                        Button::new(if border {
                            "border-palette-retry"
                        } else {
                            "photo-palette-retry"
                        })
                        .label("重新取色")
                        .small()
                        .ghost()
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.extract_photo_palette(window, cx)
                            }),
                        ),
                    )
                },
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
                            let selected = if border {
                                self.backend
                                    .params()
                                    .photo_border
                                    .palette_rank()
                                    .map(|source| source.min(swatches.len() - 1))
                                    == Some(rank)
                            } else if solid {
                                self.backend
                                    .params()
                                    .background_palette_rank
                                    .map(|source| source.min(swatches.len() - 1))
                                    == Some(rank)
                            } else {
                                self.palette_color_selected(rank)
                            };
                            let ink = if u32::from(r) + u32::from(g) + u32::from(b) > 384 {
                                black()
                            } else {
                                white()
                            };
                            Button::new((prefix, rank))
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
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if border {
                                        this.select_border_palette_color(rank, window, cx);
                                    } else if solid {
                                        this.select_solid_palette_color(rank, window, cx);
                                    } else {
                                        this.toggle_palette_color(rank, window, cx);
                                    }
                                }))
                        },
                    )))
                    .when(border, |this| {
                        this.child(description(
                            "色卡风格与背景共用；应用到其它照片时匹配对应代表色",
                        ))
                    })
                    .when(!solid && !border, |this| {
                        this.child(
                            Button::new("photo-palette-add-all")
                                .label("全部添加")
                                .small()
                                .outline()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.select_all_palette_colors(window, cx)
                                })),
                        )
                        .child(description(
                            "色卡颜色随照片匹配；应用到其它照片时，使用那张照片的对应代表色",
                        ))
                    })
                },
            )
    }
}
