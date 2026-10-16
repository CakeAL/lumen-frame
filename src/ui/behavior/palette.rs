//! 渐变编辑、照片色卡与可复用取色规则。控件身份独立于色标下标。

use super::super::{AppView, component::field::ColorField};
use crate::{
    features::photo_palette::{self, PaletteSwatch},
    watermark::{BackgroundGradient, BackgroundKind, GradientPalette, PaletteTheme},
    workspace::PhotoId,
};
use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{AnyWindowHandle, AppContext as _, Context, SharedString, Subscription, Window};
use std::collections::HashMap;

pub(in crate::ui::app) enum PaletteStatus {
    Loading,
    Ready(Vec<PaletteSwatch>),
    Failed(SharedString),
}

pub(in crate::ui::app) struct GradientStopControl {
    pub id: u64,
    pub color: ColorField,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(in crate::ui::app) struct BackgroundState {
    pub palettes: HashMap<(PhotoId, PaletteTheme), PaletteStatus>,
    pub editor: Option<u64>,
    pub colors: Vec<GradientStopControl>,
    next_color_id: u64,
    window: Option<AnyWindowHandle>,
}

impl AppView {
    pub(in crate::ui::app) fn selected_palette(&self) -> Option<&PaletteStatus> {
        self.palette_status(self.params.gradient_palette.theme())
    }

    pub(in crate::ui::app) fn palette_status(&self, theme: PaletteTheme) -> Option<&PaletteStatus> {
        self.background_state
            .palettes
            .get(&(self.selected_photo_id()?, theme))
    }

    pub(in crate::ui::app) fn photo_border_display_color(&self) -> [u8; 3] {
        if let (Some(rank), Some(PaletteStatus::Ready(swatches))) = (
            self.params.photo_border.palette_rank(),
            self.selected_palette(),
        ) {
            if let Some(swatch) = swatches.get(rank.min(swatches.len().saturating_sub(1))) {
                return swatch.rgb();
            }
        }
        self.params.photo_border.color()
    }

    pub(in crate::ui::app) fn select_border_palette_color(
        &mut self,
        rank: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PaletteStatus::Ready(swatches)) = self.selected_palette() else {
            return;
        };
        let Some(swatch) = swatches.get(rank) else {
            return;
        };
        let rgb = swatch.rgb();
        self.params.photo_border.select_palette(rank, rgb);
        self.controls.photo_border_color.sync(rgb, window, cx);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(in crate::ui::app) fn gradient_display_colors(&self) -> Vec<[u8; 3]> {
        match self.selected_palette() {
            Some(PaletteStatus::Ready(swatches)) => {
                photo_palette::gradient_colors(&self.params, swatches)
            }
            _ => self.params.gradient.colors().to_vec(),
        }
    }

    pub(in crate::ui::app) fn sync_gradient_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.background_state
            .window
            .get_or_insert(window.window_handle());
        let colors = self.gradient_display_colors();
        self.controls
            .gradient_angle
            .sync(self.params.gradient.angle(), window, cx);
        let theme = self.params.gradient_palette.theme();
        for control in [
            &self.controls.palette_theme,
            &self.controls.border_palette_theme,
        ] {
            if control.read(cx).selected_value() != Some(&theme) {
                control.update(cx, |state, cx| {
                    state.set_selected_value(&theme, window, cx);
                });
            }
        }
        self.controls
            .photo_border_color
            .sync(self.photo_border_display_color(), window, cx);
        self.background_state.colors.truncate(colors.len());
        while self.background_state.colors.len() < colors.len() {
            let id = self.background_state.next_color_id;
            self.background_state.next_color_id += 1;
            let color = ColorField::new(colors[self.background_state.colors.len()], window, cx);
            let subscriptions = color.subscribe(window, cx, move |this, rgb| {
                this.edit_gradient_color(id, rgb)
            });
            self.background_state.colors.push(GradientStopControl {
                id,
                color,
                _subscriptions: subscriptions,
            });
        }
        for (control, rgb) in self.background_state.colors.iter().zip(colors) {
            control.color.sync(rgb, window, cx);
        }
    }

    fn needs_photo_palette(&self) -> bool {
        let background = (self.params.photo_border.enabled()
            || matches!(
                self.params.background_kind(),
                BackgroundKind::Solid | BackgroundKind::Gradient
            ))
            && self.selected_photo_id().is_some()
            && self.selected_palette().is_none();
        let card = self.params.palette_card.enabled()
            && self.selected_photo_id().is_some()
            && self
                .palette_status(self.params.palette_card.theme())
                .is_none();
        background || card
    }

    fn ensure_photo_palettes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if (self.params.photo_border.enabled()
            || self.params.solid_background
            || self.params.gradient_background)
            && self.selected_palette().is_none()
        {
            self.extract_photo_palette(window, cx);
        }
        let theme = self.params.palette_card.theme();
        if self.params.palette_card.enabled() && self.palette_status(theme).is_none() {
            self.extract_palette_with_theme(theme, window, cx);
        }
    }

    /// 空队列重新导入时也要刷新颜色编辑器，不能继续显示上一张照片的已求值颜色。
    pub(in crate::ui::app) fn schedule_palette_for_selected(&mut self, cx: &mut Context<Self>) {
        if !self.needs_photo_palette() {
            return;
        }
        let Some(handle) = self.background_state.window else {
            return;
        };
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| {
                    view.update(cx, |this, cx| {
                        if this.needs_photo_palette() {
                            this.ensure_photo_palettes(window, cx);
                        }
                    })
                    .ok();
                })
                .ok();
        });
    }

    pub(in crate::ui::app) fn restore_gradient_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 旧照片的输入事件可能已排队，分配新身份以免落到新照片的同一位置。
        self.background_state.colors.clear();
        self.background_state.editor = None;
        self.sync_gradient_controls(window, cx);
        if self.needs_photo_palette() {
            self.ensure_photo_palettes(window, cx);
        }
    }

    pub(in crate::ui::app) fn extract_photo_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extract_palette_with_theme(self.params.gradient_palette.theme(), window, cx);
    }

    pub(in crate::ui::app) fn extract_palette_with_theme(
        &mut self,
        theme: PaletteTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(photo) = self.selected_photo() else {
            return;
        };
        let id = photo.id();
        let path = photo.path().to_path_buf();
        if matches!(self.palette_status(theme), Some(PaletteStatus::Loading)) {
            return;
        }
        self.background_state
            .palettes
            .insert((id, theme), PaletteStatus::Loading);
        if theme == self.params.gradient_palette.theme() {
            self.background_state.editor = None;
        }
        let handle = self
            .background_state
            .window
            .unwrap_or(window.window_handle());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { photo_palette::extract_with_theme(&path, 6, theme) })
                .await;
            handle
                .update(cx, |_, window, cx| {
                    this.update(cx, |this, cx| {
                        if this.workspace.photo(id).is_none() {
                            return;
                        }
                        let status = match result {
                            Ok(colors) => PaletteStatus::Ready(colors),
                            Err(error) => PaletteStatus::Failed(
                                format!("取色失败：{error:#}；请重试或换一张照片").into(),
                            ),
                        };
                        this.background_state.palettes.insert((id, theme), status);
                        if this.selected_photo_id() == Some(id)
                            && this.params.gradient_palette.theme() == theme
                        {
                            this.controls.photo_border_color.sync(
                                this.photo_border_display_color(),
                                window,
                                cx,
                            );
                            // 只更新派生色标，不打断角度、手动颜色或 Theme 下拉框的在途编辑。
                            let colors = this.gradient_display_colors();
                            let sources = this.params.gradient_palette.slots(colors.len());
                            for ((control, rgb), source) in
                                this.background_state.colors.iter().zip(colors).zip(sources)
                            {
                                if source.is_some() {
                                    control.color.sync(rgb, window, cx);
                                }
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .ok();
        })
        .detach();
    }

    pub(in crate::ui::app) fn set_palette_theme(
        &mut self,
        theme: PaletteTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slots = self
            .params
            .gradient_palette
            .slots(self.params.gradient.colors().len());
        self.params.gradient_palette = GradientPalette::new(theme, slots);
        self.background_state.editor = None;
        self.sync_gradient_controls(window, cx);
        if self.selected_palette().is_none() {
            self.extract_photo_palette(window, cx);
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(in crate::ui::app) fn set_gradient_angle(
        &mut self,
        angle: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.params.gradient =
            BackgroundGradient::new(angle, self.params.gradient.colors().to_vec())
                .expect("valid angle");
        self.controls
            .gradient_angle
            .sync(self.params.gradient.angle(), window, cx);
        self.refresh_preview(cx);
        cx.notify();
    }

    fn edit_gradient_color(&mut self, id: u64, rgb: [u8; 3]) {
        let Some(index) = self
            .background_state
            .colors
            .iter()
            .position(|control| control.id == id)
        else {
            return;
        };
        let mut colors = self.gradient_display_colors();
        // 外部控件同步不能把色卡来源改成手动颜色。
        if colors[index] == rgb {
            return;
        }
        let mut slots = self.params.gradient_palette.slots(colors.len());
        colors[index] = rgb;
        slots[index] = None;
        self.params.gradient =
            BackgroundGradient::new(self.params.gradient.angle(), colors).expect("valid colors");
        self.params.gradient_palette =
            GradientPalette::new(self.params.gradient_palette.theme(), slots);
    }

    fn replace_gradient_colors(
        &mut self,
        colors: Vec<[u8; 3]>,
        slots: Vec<Option<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.params.gradient = BackgroundGradient::new(self.params.gradient.angle(), colors)
            .expect("at least one color");
        self.params.gradient_palette =
            GradientPalette::new(self.params.gradient_palette.theme(), slots);
        self.params.set_background_kind(BackgroundKind::Gradient);
        self.sync_gradient_controls(window, cx);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(in crate::ui::app) fn add_gradient_color(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut colors = self.gradient_display_colors();
        let mut slots = self.params.gradient_palette.slots(colors.len());
        colors.push(*colors.last().unwrap());
        slots.push(None);
        self.replace_gradient_colors(colors, slots, window, cx);
    }

    pub(in crate::ui::app) fn remove_gradient_color(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.params.gradient.colors().len() <= 1 {
            return;
        }
        let Some(index) = self
            .background_state
            .colors
            .iter()
            .position(|control| control.id == id)
        else {
            return;
        };
        let mut colors = self.gradient_display_colors();
        let mut slots = self.params.gradient_palette.slots(colors.len());
        colors.remove(index);
        slots.remove(index);
        self.background_state.colors.remove(index);
        self.replace_gradient_colors(colors, slots, window, cx);
    }

    /// 勾选状态直接由来源规则推导，避免 UI 选择和实际渐变变成两份状态。
    pub(in crate::ui::app) fn palette_color_selected(&self, rank: usize) -> bool {
        self.params
            .gradient_palette
            .slots(self.params.gradient.colors().len())
            .contains(&Some(rank))
    }

    pub(in crate::ui::app) fn select_solid_palette_color(
        &mut self,
        rank: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PaletteStatus::Ready(swatches)) = self.selected_palette() else {
            return;
        };
        let Some(swatch) = swatches.get(rank) else {
            return;
        };
        let rgb = swatch.rgb();
        self.params.background = rgb;
        self.controls.background.sync(rgb, window, cx);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(in crate::ui::app) fn toggle_palette_color(
        &mut self,
        rank: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PaletteStatus::Ready(swatches)) = self.selected_palette() else {
            return;
        };
        let Some(swatch) = swatches.get(rank) else {
            return;
        };
        let rgb = swatch.rgb();
        let mut colors = self.gradient_display_colors();
        let mut slots = self.params.gradient_palette.slots(colors.len());
        if slots.contains(&Some(rank)) {
            for index in (0..slots.len()).rev() {
                if slots[index] == Some(rank) {
                    colors.remove(index);
                    slots.remove(index);
                    self.background_state.colors.remove(index);
                }
            }
            // 最后一个勾选取消后保留单色，解除色卡来源，保证预览始终有背景。
            if colors.is_empty() {
                colors.push(rgb);
                slots.push(None);
            }
        } else {
            // 首次选择色卡替换初始/手动颜色，随后按选择顺序加入。
            if !slots.iter().any(Option::is_some) {
                colors.clear();
                slots.clear();
                self.background_state.colors.clear();
            }
            colors.push(rgb);
            slots.push(Some(rank));
        }
        self.background_state.editor = None;
        self.replace_gradient_colors(colors, slots, window, cx);
    }

    pub(in crate::ui::app) fn select_all_palette_colors(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PaletteStatus::Ready(swatches)) = self.selected_palette() else {
            return;
        };
        if swatches.is_empty() {
            return;
        }
        let colors = swatches.iter().map(PaletteSwatch::rgb).collect();
        let slots = (0..swatches.len()).map(Some).collect();
        self.background_state.editor = None;
        self.background_state.colors.clear();
        self.replace_gradient_colors(colors, slots, window, cx);
    }

    pub(in crate::ui::app) fn swap_gradient_colors(
        &mut self,
        from: u64,
        to: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let controls = &self.background_state.colors;
        let (Some(a), Some(b)) = (
            controls.iter().position(|control| control.id == from),
            controls.iter().position(|control| control.id == to),
        ) else {
            return;
        };
        if a == b {
            return;
        }
        let mut colors = self.gradient_display_colors();
        let mut slots = self.params.gradient_palette.slots(colors.len());
        colors.swap(a, b);
        slots.swap(a, b);
        self.background_state.colors.swap(a, b);
        self.background_state.editor = None;
        self.replace_gradient_colors(colors, slots, window, cx);
    }

    pub(in crate::ui::app) fn apply_current_settings_to_all(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self.selected_photo_id() else {
            return;
        };
        let current = self.current_watermark(cx);
        for photo in self.workspace.photos() {
            if photo.id() == selected {
                continue;
            }
            let text = self.watermark_for_photo(photo.id(), cx).params.custom_text;
            let mut copy = current.clone();
            copy.params.custom_text = text;
            self.photo_watermarks.insert(photo.id(), copy);
        }
        self.global_watermark = current;
        self.global_watermark.params.custom_text.clear();
        window.push_notification(
            Notification::success(format!("已应用到全部 {} 张照片", self.workspace.len())),
            cx,
        );
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Entity, ScrollDelta, TestAppContext, VisualTestContext, point, px};
    use std::{cell::RefCell, rc::Rc, time::Duration};

    fn workspace(cx: &mut TestAppContext) -> (Entity<AppView>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let slot = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let slot = slot.clone();
            move |window, cx| {
                let view = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                slot.borrow_mut().replace(view.clone());
                Root::new(view, window, cx)
            }
        });
        let view = slot.borrow().clone().unwrap();
        view.update_in(cx, |view, _, cx| {
            view.add_photos(
                vec![
                    "test_images/DSC_4587.jpg".into(),
                    "test_images/ultra_hdr.jpg".into(),
                ],
                cx,
            )
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.within("background-mode").click(2_usize, cx);
            window.render_frame(cx);
            window.within("background-mode").scroll(
                2_usize,
                ScrollDelta::Pixels(point(px(0.), px(-350.))),
                cx,
            );
            window.render_frame(cx);
        });
        (view, cx)
    }

    fn settle_palette(view: &Entity<AppView>, cx: &mut VisualTestContext) {
        for _ in 0..40 {
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            if view.read_with(cx, |view, _| {
                matches!(view.selected_palette(), Some(PaletteStatus::Ready(_)))
            }) {
                return;
            }
        }
        panic!("色卡未能生成");
    }

    #[gpui_kit::test]
    fn solid_background_uses_single_palette_color(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx);
        let gradient = view.read_with(cx, |view, _| view.params.gradient.clone());
        cx.update(|window, cx| {
            window.within("background-mode").click(1_usize, cx);
            assert!(window.try_find("photo-palette-extract").is_none());
        });
        settle_palette(&view, cx);
        cx.update(|window, cx| {
            assert!(window.try_find("photo-palette-add-all").is_none());
            window.click(("photo-palette-color", 0_usize), cx);
            window.click(("photo-palette-color", 2_usize), cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let Some(PaletteStatus::Ready(swatches)) = view.selected_palette() else {
                panic!("缺少色卡");
            };
            assert_eq!(view.params.background_kind(), BackgroundKind::Solid);
            assert_eq!(view.params.background, swatches[2].rgb());
            assert_eq!(view.params.gradient, gradient);
        });
        cx.update(|window, cx| {
            window.within("background-mode").click(2_usize, cx);
            window.within("background-mode").click(1_usize, cx);
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| matches!(
            view.selected_palette(),
            Some(PaletteStatus::Ready(_))
        )));
    }

    #[gpui_kit::test]
    fn gradient_direction_grid_and_hover_color_editor_use_real_controls(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx);
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient.angle()),
            135.0
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient.colors().len()),
            2
        );
        cx.update(|window, cx| {
            assert!(window.try_find("background-gradient-css").is_none());
            let up_left = window.find(("gradient-direction", 315_usize)).bounds();
            let up = window.find(("gradient-direction", 0_usize)).bounds();
            let down = window.find(("gradient-direction", 180_usize)).bounds();
            assert_eq!(up_left.origin.y, up.origin.y);
            assert_eq!(up.origin.x, down.origin.x);
            assert!(down.origin.y > up.origin.y);
            window.click("gradient-direction-center", cx);
            let input = window
                .within("gradient-angle")
                .find("number-input")
                .bounds();
            assert!(input.right() < up_left.left());
            window.click(("gradient-direction", 90_usize), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient.angle()),
            90.0
        );
        let first = view.read_with(cx, |view, _| view.background_state.colors[0].id);
        cx.update(|window, cx| {
            window.hover(("gradient-color-swatch", first), cx);
            window.render_frame(cx);
            window
                .within(("gradient-color-actions", first))
                .click("gradient-color-hex", cx);
            window.press("cmd-a", cx);
            window.input("#123456", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient.colors()[0]),
            [0x12, 0x34, 0x56]
        );
        cx.update(|window, cx| window.click("gradient-color-add", cx));
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient.colors().len()),
            3
        );
        cx.update(|window, cx| {
            window.hover(("gradient-color-swatch", first), cx);
            window.click(("gradient-color-remove", first), cx);
        });
        cx.run_until_parked();
        let remaining = view.read_with(cx, |view, _| view.background_state.colors[0].id);
        assert_ne!(first, remaining);
        cx.update(|window, cx| {
            window.hover(("gradient-color-swatch", remaining), cx);
            window.click(("gradient-color-remove", remaining), cx);
        });
        let remaining = view.read_with(cx, |view, _| view.background_state.colors[0].id);
        cx.update(|window, cx| {
            window.hover(("gradient-color-swatch", remaining), cx);
            window.click(("gradient-color-remove", remaining), cx);
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient.colors().len()),
            1
        );
        // 色标超过 16 个仍可添加，且删除其它色标不会改变剩余控件身份。
        view.update_in(cx, |view, window, cx| {
            for _ in 0..17 {
                view.add_gradient_color(window, cx);
            }
            assert_eq!(view.params.gradient.colors().len(), 18);
            assert_eq!(view.background_state.colors[0].id, remaining);
        });
    }

    #[gpui_kit::test]
    fn palette_selection_drag_and_edit_keep_sources_in_sync(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx);
        settle_palette(&view, cx);
        cx.update(|window, cx| {
            window.click(("photo-palette-color", 0_usize), cx);
            window.click(("photo-palette-color", 2_usize), cx);
            window.click(("photo-palette-color", 4_usize), cx);
        });
        cx.run_until_parked();
        let ids = view.read_with(cx, |view, _| {
            assert_eq!(
                view.params.gradient_palette.slots(3),
                [Some(0), Some(2), Some(4)]
            );
            view.background_state
                .colors
                .iter()
                .map(|control| control.id)
                .collect::<Vec<_>>()
        });
        cx.update(|window, cx| {
            window.drag_to(
                ("gradient-color-swatch", ids[0]),
                ("gradient-color-swatch", ids[2]),
                cx,
            );
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.params.gradient_palette.slots(3),
                [Some(4), Some(2), Some(0)]
            );
            assert_eq!(view.background_state.colors[2].id, ids[0]);
            for rank in [0, 2, 4] {
                assert!(view.palette_color_selected(rank));
            }
        });
        cx.update(|window, cx| {
            window.hover(("gradient-color-swatch", ids[0]), cx);
            window
                .within(("gradient-color-actions", ids[0]))
                .click("gradient-color-hex", cx);
            window.press("cmd-a", cx);
            window.input("#123456", cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.params.gradient.colors()[2], [0x12, 0x34, 0x56]);
            assert_eq!(
                view.params.gradient_palette.slots(3),
                [Some(4), Some(2), None]
            );
            assert!(!view.palette_color_selected(0));
        });
        cx.update(|window, cx| {
            window.press("escape", cx);
            window.click(("photo-palette-color", 2_usize), cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.params.gradient_palette.slots(2), [Some(4), None]);
            assert!(!view.palette_color_selected(2));
        });
        cx.update(|window, cx| window.click("photo-palette-add-all", cx));
        cx.run_until_parked();
        let count = view.read_with(cx, |view, _| {
            let Some(PaletteStatus::Ready(swatches)) = view.selected_palette() else {
                panic!()
            };
            let count = swatches.len();
            assert_eq!(view.params.gradient.colors().len(), count);
            for rank in 0..count {
                assert!(view.palette_color_selected(rank));
            }
            count
        });
        cx.update(|window, cx| window.click("photo-palette-add-all", cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient.colors().len()),
            count
        );
    }

    #[gpui_kit::test]
    fn themed_palette_multiselect_and_apply_all_keep_per_photo_color_rules(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = workspace(cx);
        cx.update(|window, cx| {
            window.click("photo-palette-theme", cx);
            window.press("down", cx);
            window.press("enter", cx);
        });
        settle_palette(&view, cx);
        assert_eq!(
            view.read_with(cx, |view, _| view.params.gradient_palette.theme()),
            PaletteTheme::Basic
        );
        cx.update(|window, cx| {
            window.scroll(
                "photo-palette-theme",
                ScrollDelta::Pixels(point(px(0.), px(-220.))),
                cx,
            );
            window.render_frame(cx);
            window.click(("photo-palette-color", 0_usize), cx);
            window.click(("photo-palette-color", 2_usize), cx);
            assert!(window.try_find("photo-palette-add-selected").is_none());
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.params.gradient.colors().len(), 2);
            assert_eq!(view.params.gradient_palette.slots(2), [Some(0), Some(2)]);
        });
        view.update_in(cx, |view, _, _| {
            view.params.custom_text = "第一张独立文字".into()
        });
        view.update_in(cx, |view, window, cx| {
            view.select_photo_at(1, window, cx);
            view.params.custom_text = "第二张独立文字".into();
            view.select_photo_at(0, window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.click("apply-settings-to-all", cx));
        let first_id = view.read_with(cx, |view, _| view.selected_photo_id().unwrap());
        view.update_in(cx, |view, window, cx| view.select_photo_at(1, window, cx));
        settle_palette(&view, cx);
        view.read_with(cx, |view, cx| {
            assert_eq!(view.params.custom_text, "第二张独立文字");
            assert_eq!(view.params.gradient_palette.slots(2), [Some(0), Some(2)]);
            assert_eq!(view.params.gradient_palette.theme(), PaletteTheme::Basic);
            let display = view.gradient_display_colors();
            let Some(PaletteStatus::Ready(swatches)) = view.selected_palette() else {
                panic!()
            };
            assert_eq!(display[0], swatches[0].rgb());
            assert_eq!(display[1], swatches[2.min(swatches.len() - 1)].rgb());
            let first = photo_palette::resolve(
                view.workspace.photo(first_id).unwrap().path(),
                &view.watermark_for_photo(first_id, cx).params,
            )
            .unwrap();
            assert_ne!(display[0], first.gradient.colors()[0]);
            // 异步同步颜色编辑器不能把动态来源误改为固定颜色。
            assert_eq!(view.params.gradient_palette.slots(2)[0], Some(0));
        });
        view.update_in(cx, |view, window, cx| {
            let Some(PaletteStatus::Ready(swatches)) = view.selected_palette() else {
                panic!()
            };
            let count = swatches.len();
            view.select_all_palette_colors(window, cx);
            assert_eq!(view.params.gradient.colors().len(), count);
            assert_eq!(
                view.params.gradient_palette.slots(count),
                (0..count).map(Some).collect::<Vec<_>>()
            );
            view.clear_photos(window, cx);
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.background_state.palettes.is_empty()));
        view.update_in(cx, |view, _, cx| {
            view.add_photos(vec!["test_images/ultra_hdr.jpg".into()], cx)
        });
        settle_palette(&view, cx);
        view.read_with(cx, |view, _| {
            let Some(PaletteStatus::Ready(swatches)) = view.selected_palette() else {
                panic!()
            };
            assert_eq!(view.gradient_display_colors()[0], swatches[0].rgb());
            assert_eq!(view.params.gradient_palette.slots(2)[0], Some(0));
        });
    }
}
