//! 照片色卡的入口和独立编辑窗口，业务值始终由 AppView 的参数持有。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Entity, IntoElement, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

use super::{
    super::{AppView, behavior::palette::PaletteStatus},
    background::PALETTE_THEMES,
    field::{
        Choice, ColorField, NumberField, choices, description, field, index_of, rgb_to_hsla,
        select_state, warning,
    },
    inspector::POSITIONS,
};
use crate::backend::watermark::{
    GroupAttachment, PaletteCard, PaletteShape, PaletteTheme, Placement, TextAlign, TextDirection,
};

type NumberSpec = (
    &'static str,
    &'static str,
    fn(&PaletteCard) -> f64,
    fn(&mut PaletteCard, f64),
    f64,
    f64,
);
const NUMBERS: &[NumberSpec] = &[
    (
        "attachment-gap",
        "组间距",
        PaletteCard::attachment_gap,
        PaletteCard::set_attachment_gap,
        0.0,
        20.0,
    ),
    (
        "width",
        "色块宽度",
        PaletteCard::width,
        PaletteCard::set_width,
        0.5,
        40.0,
    ),
    (
        "size",
        "色块大小 / 色条高度",
        PaletteCard::size,
        PaletteCard::set_size,
        0.5,
        20.0,
    ),
    (
        "length",
        "色条长度",
        PaletteCard::length,
        PaletteCard::set_length,
        5.0,
        200.0,
    ),
    (
        "gap",
        "颜色间距",
        PaletteCard::gap,
        PaletteCard::set_gap,
        0.0,
        10.0,
    ),
    (
        "radius",
        "圆角",
        PaletteCard::radius,
        PaletteCard::set_radius,
        0.0,
        10.0,
    ),
    (
        "padding",
        "与图片边缘留白",
        PaletteCard::padding,
        PaletteCard::set_padding,
        0.0,
        20.0,
    ),
    (
        "label-size",
        "色号字号",
        PaletteCard::label_size,
        PaletteCard::set_label_size,
        0.5,
        5.0,
    ),
];

struct NumberControl {
    spec: &'static NumberSpec,
    field: NumberField,
}

pub(in crate::app) struct PaletteCardEditor {
    numbers: Vec<NumberControl>,
    theme: Entity<SelectState<Vec<Choice<PaletteTheme>>>>,
    label_color: ColorField,
    window: Option<AnyWindowHandle>,
    opening: bool,
    _subscriptions: Vec<Subscription>,
}

impl PaletteCardEditor {
    pub(in crate::app) fn new(
        card: &PaletteCard,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let numbers = NUMBERS
            .iter()
            .map(|spec| {
                let field = NumberField::new(
                    (spec.2)(card),
                    spec.4,
                    spec.5,
                    0.1,
                    1,
                    100.0,
                    "%",
                    window,
                    cx,
                );
                subscriptions.extend(field.subscribe(window, cx, move |this, value| {
                    (spec.3)(&mut this.backend.params_mut().palette_card, value)
                }));
                NumberControl { spec, field }
            })
            .collect();
        let theme = select_state(
            choices(PALETTE_THEMES),
            index_of(PALETTE_THEMES, &card.theme()),
            window,
            cx,
        );
        subscriptions.push(
            cx.subscribe_in(&theme, window, |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(theme)) = event {
                    this.backend.params_mut().palette_card.set_theme(*theme);
                    if this.palette_status(*theme).is_none() {
                        this.extract_palette_with_theme(*theme, window, cx);
                    }
                    this.refresh_preview(cx);
                    cx.notify();
                }
            }),
        );
        let label_color = ColorField::new(card.label_color(), window, cx);
        subscriptions.extend(label_color.subscribe(window, cx, |this, rgb| {
            this.backend.params_mut().palette_card.set_label_color(rgb)
        }));
        Self {
            numbers,
            theme,
            label_color,
            window: None,
            opening: false,
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::app) fn sync(&self, card: &PaletteCard, window: &mut Window, cx: &mut App) {
        for control in &self.numbers {
            control.field.sync((control.spec.2)(card), window, cx);
        }
        self.label_color.sync(card.label_color(), window, cx);
        self.theme.update(cx, |state, cx| {
            state.set_selected_value(&card.theme(), window, cx)
        });
    }
}

struct PaletteCardWindow {
    app: Entity<AppView>,
    _subscription: Subscription,
}

impl Render for PaletteCardWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .child(crate::window_effects::title_bar(cx).child(div().text_sm().child("照片色卡")))
            .child(
                div()
                    .id("palette-card-editor-scroll")
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .overflow_y_scrollbar()
                    .child(
                        self.app
                            .read(cx)
                            .render_palette_card_editor(self.app.clone(), cx),
                    ),
            )
    }
}

fn tabs<T: Copy + PartialEq + 'static>(
    id: &'static str,
    entries: &'static [(&'static str, T)],
    current: T,
    view: Entity<AppView>,
    set: fn(&mut PaletteCard, T),
) -> impl IntoElement {
    TabBar::new(id)
        .segmented()
        .w_full()
        .children(
            entries
                .iter()
                .map(|(label, _)| Tab::new().label(*label).flex_1()),
        )
        .selected_index(
            entries
                .iter()
                .position(|(_, value)| *value == current)
                .unwrap_or(0),
        )
        .on_click(move |ix: &usize, _, cx| {
            view.update(cx, |this, cx| {
                set(&mut this.backend.params_mut().palette_card, entries[*ix].1);
                this.refresh_preview(cx);
                cx.notify();
            });
        })
}

impl AppView {
    fn set_palette_attachment(
        &mut self,
        target: Option<u64>,
        side: Placement,
        cx: &mut Context<Self>,
    ) {
        let gap = self.backend.params().palette_card.attachment_gap();
        // 菜单捕获稳定 ID，在操作发生时再映射成当前快照序号。
        let attachment = target
            .and_then(|id| self.text_groups.iter().position(|g| g.id == id))
            .map(|target| GroupAttachment { target, side, gap });
        self.backend
            .params_mut()
            .palette_card
            .set_attachment(attachment);
        self.refresh_preview(cx);
        cx.notify();
    }

    fn render_palette_attachment(&self, view: Entity<Self>, cx: &App) -> impl IntoElement {
        let attachment = self.backend.params().palette_card.attachment();
        let target = attachment
            .and_then(|a| self.text_groups.get(a.target))
            .map(|g| g.id);
        let side = attachment.map(|a| a.side).unwrap_or(Placement::Right);
        let label = attachment
            .filter(|a| a.target < self.text_groups.len())
            .map(|a| format!("文字组 {}", a.target + 1))
            .unwrap_or_else(|| "独立位置".into());
        let targets = self
            .text_groups
            .iter()
            .enumerate()
            .map(|(ix, g)| (g.id, format!("文字组 {}", ix + 1)))
            .collect::<Vec<_>>();
        let side_view = view.clone();
        v_flex()
            .w_full()
            .gap_4()
            .child(field(
                "紧贴文字组",
                Button::new("palette-attachment-target")
                    .label(label)
                    .outline()
                    .dropdown_caret(true)
                    .w_full()
                    .dropdown_menu(move |menu, _, _| {
                        let independent = view.clone();
                        let menu = menu.item(
                            PopupMenuItem::new("独立位置")
                                .checked(target.is_none())
                                .on_click(move |_, _, cx| {
                                    independent.update(cx, |this, cx| {
                                        this.set_palette_attachment(None, side, cx)
                                    });
                                }),
                        );
                        targets.iter().fold(menu, |menu, (id, label)| {
                            let id = *id;
                            let view = view.clone();
                            menu.item(
                                PopupMenuItem::new(label.clone())
                                    .checked(target == Some(id))
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.set_palette_attachment(Some(id), side, cx)
                                        });
                                    }),
                            )
                        })
                    }),
                cx,
            ))
            .when(target.is_some(), |el| {
                el.child(field(
                    "相邻方向",
                    tabs(
                        "palette-attachment-side",
                        &[
                            ("左侧", Placement::Left),
                            ("右侧", Placement::Right),
                            ("上方", Placement::Up),
                            ("下方", Placement::Bottom),
                        ],
                        side,
                        side_view,
                        |card, side| {
                            if let Some(mut attachment) = card.attachment().cloned() {
                                attachment.side = side;
                                card.set_attachment(Some(attachment));
                            }
                        },
                    ),
                    cx,
                ))
                .child(description(
                    "位置和对齐跟随目标组；目标组为空时恢复独立位置。",
                ))
            })
    }

    pub(super) fn render_palette_card_section(&self, cx: &Context<Self>) -> impl IntoElement {
        let card = &self.backend.params().palette_card;
        GroupBox::new()
            .id("palette-card-section")
            .title("照片色卡")
            .child(if card.enabled() {
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new("palette-card-open")
                            .label("编辑色卡…")
                            .flex_1()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_palette_card(window, cx)
                            })),
                    )
                    .child(
                        Button::new("palette-card-remove")
                            .icon(gpui_kit::assets::IconName::Trash)
                            .ghost()
                            .tooltip("移除色卡")
                            .accessibility_label("移除色卡")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.backend.params_mut().palette_card.set_enabled(false);
                                this.refresh_preview(cx);
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            } else {
                Button::new("palette-card-add")
                    .icon(gpui_kit::assets::IconName::Plus)
                    .label("添加色卡…")
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.backend.params_mut().palette_card.set_enabled(true);
                        this.open_palette_card(window, cx);
                        this.refresh_preview(cx);
                        cx.notify();
                    }))
                    .into_any_element()
            })
    }

    fn render_palette_card_editor(&self, view: Entity<Self>, cx: &App) -> impl IntoElement {
        let card = &self.backend.params().palette_card;
        let controls = &self.palette_card_editor;
        let status = self.palette_status(card.theme());
        let labels = card.show_hex() || card.show_ratio();
        let aligns = if matches!(card.position(), Placement::Left | Placement::Right) {
            &[
                ("上侧", TextAlign::Left),
                ("居中", TextAlign::Center),
                ("下侧", TextAlign::Right),
            ] as &'static [(&str, TextAlign)]
        } else {
            &[
                ("左侧", TextAlign::Left),
                ("居中", TextAlign::Center),
                ("右侧", TextAlign::Right),
            ]
        };
        let enable_view = view.clone();
        let all_view = view.clone();
        let clear_view = view.clone();
        let retry_view = view.clone();
        let proportional_view = view.clone();
        let hex_view = view.clone();
        let ratio_view = view.clone();
        v_flex()
            .w_full()
            .gap_4()
            .when_some(self.selected_photo(), |el, photo| {
                el.child(description(format!("当前照片：{}", photo.path().file_name().unwrap_or_default().to_string_lossy())))
            })
            .child(
                Switch::new("palette-card-enabled")
                    .label("显示照片色卡")
                    .checked(card.enabled())
                    .on_change(move |enabled, window, cx| {
                        enable_view.update(cx, |this, cx| {
                            this.backend.params_mut().palette_card.set_enabled(*enabled);
                            if *enabled
                                && this
                                    .palette_status(this.backend.params().palette_card.theme())
                                    .is_none()
                            {
                                this.extract_palette_with_theme(
                                    this.backend.params().palette_card.theme(),
                                    window,
                                    cx,
                                );
                            }
                            this.refresh_preview(cx);
                            cx.notify();
                        });
                    }),
            )
            .child(field(
                "取色风格",
                Select::new(&controls.theme)
                    .id("palette-card-theme")
                    .w_full(),
                cx,
            ))
            .when(self.selected_photo_id().is_none(), |el| {
                el.child(description("添加照片后显示可选颜色。"))
            })
            .when(matches!(status, Some(PaletteStatus::Loading)), |el| {
                el.child(description("正在加载照片色卡…"))
            })
            .when_some(
                if let Some(PaletteStatus::Failed(error)) = status {
                    Some(error.clone())
                } else {
                    None
                },
                |el, error| {
                    el.child(warning(error, cx)).child(
                        Button::new("palette-card-retry")
                            .label("重新取色")
                            .outline()
                            .small()
                            .on_click(move |_, window, cx| {
                                retry_view.update(cx, |this, cx| {
                                    this.extract_palette_with_theme(
                                        this.backend.params().palette_card.theme(),
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    )
                },
            )
            .when_some(
                if let Some(PaletteStatus::Ready(colors)) = status {
                    Some(colors)
                } else {
                    None
                },
                |el, colors| {
                    let count = colors.len();
                    el.child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .justify_between()
                                    .child("选用颜色")
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Button::new("palette-card-all")
                                                    .label("全选")
                                                    .ghost()
                                                    .small()
                                                    .on_click(move |_, _, cx| {
                                                        all_view.update(cx, |this, cx| {
                                                            this.backend.params_mut()
                                                                .palette_card
                                                                .set_selected((0..count).collect());
                                                            this.refresh_preview(cx);
                                                            cx.notify();
                                                        });
                                                    }),
                                            )
                                            .child(
                                                Button::new("palette-card-clear")
                                                    .label("清空")
                                                    .ghost()
                                                    .small()
                                                    .on_click(move |_, _, cx| {
                                                        clear_view.update(cx, |this, cx| {
                                                            this.backend.params_mut()
                                                                .palette_card
                                                                .set_selected(Vec::new());
                                                            this.refresh_preview(cx);
                                                            cx.notify();
                                                        });
                                                    }),
                                            ),
                                    ),
                            )
                            .child(h_flex().gap_2().flex_wrap().children(
                                colors.iter().enumerate().map(|(rank, swatch)| {
                                    let view = view.clone();
                                    let [r, g, b] = swatch.rgb();
                                    v_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            Button::new(("palette-card-color", rank))
                                                .size_10()
                                                .p_0()
                                                .outline()
                                                .selected(card.selected().contains(&rank))
                                                .accessibility_label(format!(
                                                    "选择颜色 #{r:02X}{g:02X}{b:02X}"
                                                ))
                                                .tooltip(format!(
                                                    "#{r:02X}{g:02X}{b:02X} · {:.1}%",
                                                    swatch.ratio() * 100.0
                                                ))
                                                .child(
                                                    div()
                                                        .size_6()
                                                        .rounded(cx.theme().radius)
                                                        .bg(rgb_to_hsla(swatch.rgb()))
                                                        .flex().items_center().justify_center()
                                                        .when(card.selected().contains(&rank), |el| {
                                                            // 色块本身是图像数据，勾选符号按其亮度取对比色。
                                                            el.child(gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Check)
                                                                .size_4().text_color(if u32::from(r)+u32::from(g)+u32::from(b) > 384 { gpui_kit::black() } else { gpui_kit::white() }))
                                                        }),
                                                )
                                                .on_click(move |_, _, cx| {
                                                    view.update(cx, |this, cx| {
                                                        this.backend.params_mut().palette_card.toggle(rank);
                                                        this.refresh_preview(cx);
                                                        cx.notify();
                                                    });
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(format!("{:.1}%", swatch.ratio() * 100.0)),
                                        )
                                }),
                            ))
                            .child(description(
                                "占比为该颜色在照片中的估计面积；按点击顺序排列。",
                            ))
                            .when(!card.selected().iter().any(|rank| *rank < count), |el| {
                                el.child(description("尚未选用颜色，照片中暂不显示色卡。"))
                            }),
                    )
                },
            )
            .child(self.render_palette_attachment(view.clone(), cx))
            .when(card.attachment().is_none(), |el| el.child(field(
                "位置",
                tabs(
                    "palette-card-position",
                    POSITIONS,
                    card.position(),
                    view.clone(),
                    PaletteCard::set_position,
                ),
                cx,
            ))
            .child(field(
                "对齐",
                tabs(
                    "palette-card-align",
                    aligns,
                    card.align(),
                    view.clone(),
                    PaletteCard::set_align,
                ),
                cx,
            )))
            .child(field(
                "方向",
                tabs(
                    "palette-card-direction",
                    &[
                        ("横排", TextDirection::Horizontal),
                        ("竖排（旋转 90°）", TextDirection::Vertical),
                    ],
                    card.direction(),
                    view.clone(),
                    PaletteCard::set_direction,
                ),
                cx,
            ))
            .child(field(
                "形状",
                tabs(
                    "palette-card-shape",
                    &[
                        ("方形", PaletteShape::Square),
                        ("圆形", PaletteShape::Circle),
                        ("圆角矩形", PaletteShape::Rounded),
                    ],
                    card.shape(),
                    view,
                    PaletteCard::set_shape,
                ),
                cx,
            ))
            .child(
                Switch::new("palette-card-proportional")
                    .label("按占比分配色条宽度")
                    .checked(card.proportional())
                    .disabled(card.shape() == PaletteShape::Circle)
                    .on_change(move |value, _, cx| {
                        proportional_view.update(cx, |this, cx| {
                            this.backend.params_mut().palette_card.set_proportional(*value);
                            this.refresh_preview(cx);
                            cx.notify();
                        });
                    }),
            )
            .child(description(
                "圆形保持等大；色条按选中颜色的占比归一化，色号在下方等距显示。",
            ))
            .child(
                h_flex()
                    .gap_4()
                    .child(
                        Switch::new("palette-card-show-hex")
                            .label("显示色号")
                            .checked(card.show_hex())
                            .on_change(move |value, _, cx| {
                                hex_view.update(cx, |this, cx| {
                                    this.backend.params_mut().palette_card.set_show_hex(*value);
                                    this.refresh_preview(cx);
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Switch::new("palette-card-show-ratio")
                            .label("显示占比")
                            .checked(card.show_ratio())
                            .on_change(move |value, _, cx| {
                                ratio_view.update(cx, |this, cx| {
                                    this.backend.params_mut().palette_card.set_show_ratio(*value);
                                    this.refresh_preview(cx);
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .children(
                controls
                    .numbers
                    .iter()
                    .filter(|c| match c.spec.0 {
                        "width" => card.shape() == PaletteShape::Rounded && !card.proportional(),
                        "length" => card.proportional(),
                        "radius" => card.shape() == PaletteShape::Rounded,
                        "label-size" => labels,
                        "attachment-gap" => card.attachment().is_some(),
                        "padding" => card.attachment().is_none() && card.align() != TextAlign::Center,
                        _ => true,
                    })
                    .map(|control| {
                        div().id(control.spec.0).child(control.field.render(
                            control.spec.1,
                            false,
                            cx,
                        ))
                    }),
            )
            .when(labels, |el| {
                el.child(field("色号颜色", controls.label_color.render_control(), cx))
            })
            .child(description(
                "尺寸以照片高度的百分比计算。色卡与文字组共用位置和对齐逻辑。",
            ))
    }

    pub(super) fn open_palette_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.backend.params().palette_card.theme();
        if self.palette_status(theme).is_none() {
            self.extract_palette_with_theme(theme, window, cx);
        }
        if let Some(handle) = self.palette_card_editor.window.take()
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            self.palette_card_editor.window = Some(handle);
            return;
        }
        if self.palette_card_editor.opening {
            return;
        }
        self.palette_card_editor.opening = true;
        let view = cx.entity();
        cx.defer(move |cx| {
            if !view.read(cx).palette_card_editor.opening {
                return;
            }
            // 窗口边界使用平台逻辑像素，内部控件跟随主题的 rem 缩放。
            let bounds = Bounds::centered(None, size(px(620.), px(820.)), cx);
            let app = view.clone();
            let result = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(500.), px(500.))),
                    ..TitleBar::window_options()
                },
                cx,
                move |_, cx| {
                    cx.new(|cx| {
                        let subscription = cx.observe(&app, |_, _, cx| cx.notify());
                        PaletteCardWindow {
                            app,
                            _subscription: subscription,
                        }
                    })
                },
            );
            view.update(cx, |this, cx| {
                this.palette_card_editor.opening = false;
                match result {
                    Ok((handle, _)) => this.palette_card_editor.window = Some(handle),
                    Err(error) => eprintln!("无法打开色卡编辑窗口：{error:#}"),
                }
                cx.notify();
            });
        });
    }

    #[cfg(test)]
    pub(in crate::app) fn palette_editor_window(&self) -> Option<AnyWindowHandle> {
        self.palette_card_editor.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, component::Root, test::TestWindowExt as _};

    #[gpui_kit::test]
    fn palette_card_controls_select_colors_and_keep_per_photo_settings(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (root, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
            let editor = cx.new(|cx| {
                let subscription = cx.observe(&app, |_, _, cx| cx.notify());
                PaletteCardWindow {
                    app,
                    _subscription: subscription,
                }
            });
            Root::new(editor, window, cx)
        });
        let app = root.read_with(cx, |root, cx| {
            root.view()
                .clone()
                .downcast::<PaletteCardWindow>()
                .unwrap()
                .read(cx)
                .app
                .clone()
        });
        let colors = crate::backend::photo_palette::extract(
            std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../test_images/DSC_4587.jpg"
            )),
            6,
        )
        .unwrap();
        app.update_in(cx, |app, _, cx| {
            app.add_photos(
                vec![
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../test_images/DSC_4587.jpg"
                    )
                    .into(),
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../test_images/ultra_hdr.jpg"
                    )
                    .into(),
                ],
                cx,
            );
            app.backend.params_mut().palette_card.set_enabled(true);
            app.background_state.palettes.insert(
                (app.selected_photo_id().unwrap(), PaletteTheme::Default),
                PaletteStatus::Ready(colors),
            );
            cx.notify();
        });
        cx.simulate_resize(size(px(700.), px(1500.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let first = window.find(("palette-card-color", 0_usize)).bounds();
            let second = window.find(("palette-card-color", 1_usize)).bounds();
            assert_eq!(first.size, second.size, "勾选状态不应改变色块按钮的尺寸");
            window.click("palette-card-clear", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.click(("palette-card-color", 2_usize), cx);
            window.click(("palette-card-color", 0_usize), cx);
            window.within("palette-card-shape").click(2_usize, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.click("palette-card-proportional", cx);
            window.within("palette-card-position").click(3_usize, cx);
            window.within("palette-card-align").click(2_usize, cx);
            window.click("palette-card-show-hex", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.click("palette-attachment-target", cx);
            window.press("up", cx);
            window.press("enter", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(window.try_find("palette-card-position").is_none());
            window.within("palette-attachment-side").click(1_usize, cx);
        });
        cx.run_until_parked();
        app.update_in(cx, |app, window, cx| {
            let current = app.current_watermark(cx);
            let card = &current.params.palette_card;
            assert_eq!(card.selected(), [2, 0]);
            assert_eq!(card.shape(), PaletteShape::Rounded);
            assert!(card.proportional() && card.show_hex());
            assert_eq!(card.position(), Placement::Left);
            assert_eq!(card.align(), TextAlign::Right);
            assert_eq!(card.attachment().unwrap().target, 0);
            assert_eq!(card.attachment().unwrap().side, Placement::Right);
            app.apply_current_settings_to_all(window, cx);
            app.select_photo_at(1, window, cx);
            assert_eq!(
                app.backend.params().palette_card,
                current.params.palette_card
            );
            let first = crate::backend::photo_palette::resolve(
                std::path::Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../test_images/DSC_4587.jpg"
                )),
                &current.params,
            )
            .unwrap();
            let second = crate::backend::photo_palette::resolve(
                std::path::Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../test_images/ultra_hdr.jpg"
                )),
                app.backend.params(),
            )
            .unwrap();
            assert_ne!(first.palette_card.colors(), second.palette_card.colors());
            app.remove_text_group(app.text_groups[0].id, cx);
            assert!(app.backend.params().palette_card.attachment().is_none());
        });
    }
}
