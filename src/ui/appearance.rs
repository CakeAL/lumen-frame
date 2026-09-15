//! 应用外观偏好：主题槽位、明暗模式、界面缩放与照片展示背景。

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Context, SharedString, Window, WindowAppearance};

use crate::config::{self, AppearanceMode};

use super::{AppView, settings};

impl AppView {
    pub fn interface_scale(&self) -> f32 {
        self.interface_scale
    }

    pub fn light_theme_name(&self) -> Option<String> {
        self.light_theme.as_ref().map(|name| name.to_string())
    }

    pub fn dark_theme_name(&self) -> Option<String> {
        self.dark_theme.as_ref().map(|name| name.to_string())
    }

    pub fn set_interface_scale(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.interface_scale = scale;
        settings::apply_interface_scale(scale, window, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn set_preview_background(&mut self, rgb: [u8; 3]) {
        if self.preview_background != rgb {
            self.preview_background = rgb;
            self.persist_settings_inner();
        }
    }

    pub(super) fn apply_theme_slots(&self, cx: &mut App) {
        for name in [&self.light_theme, &self.dark_theme].into_iter().flatten() {
            if let Some(config) = crate::theme::find(name, cx) {
                Theme::global_mut(cx).apply_config(&config);
            }
        }
    }

    fn persist_settings(&mut self, _: &App) {
        self.persist_settings_inner();
    }

    fn persist_settings_inner(&mut self) {
        let settings = config::AppSettings {
            appearance: self.appearance,
            light_theme: self.light_theme.as_ref().map(|name| name.to_string()),
            dark_theme: self.dark_theme.as_ref().map(|name| name.to_string()),
            interface_scale: Some(self.interface_scale),
            preview_background: self.preview_background,
        };
        self.settings_feedback = config::save_settings(&settings)
            .err()
            .map(|error| format!("偏好没能保存：{error:#}").into());
    }

    pub fn reset_appearance_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance = AppearanceMode::System;
        self.light_theme = None;
        self.dark_theme = None;
        self.interface_scale = settings::DEFAULT_INTERFACE_SCALE;
        self.preview_background = config::DEFAULT_PREVIEW_BACKGROUND;
        let (light, dark) = {
            let registry = ThemeRegistry::global(cx);
            (
                registry.default_light_theme().clone(),
                registry.default_dark_theme().clone(),
            )
        };
        Theme::global_mut(cx).apply_config(&light);
        Theme::global_mut(cx).apply_config(&dark);
        let (light_name, dark_name) = (light.name.clone(), dark.name.clone());
        self.settings.light_theme.update(cx, |state, cx| {
            state.set_selected_value(&light_name, window, cx)
        });
        self.settings.dark_theme.update(cx, |state, cx| {
            state.set_selected_value(&dark_name, window, cx)
        });
        self.settings
            .preview_background
            .sync(self.preview_background, window, cx);
        settings::apply_interface_scale(self.interface_scale, window, cx);
        self.apply_appearance(window, cx);
        self.persist_settings(cx);
        self.settings_feedback = Some("已恢复默认外观。".into());
        cx.notify();
    }

    pub fn set_theme_slot(
        &mut self,
        mode: ThemeMode,
        name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match mode {
            ThemeMode::Light => self.light_theme = Some(name),
            ThemeMode::Dark => self.dark_theme = Some(name),
        }
        self.apply_theme_slots(cx);
        self.apply_appearance(window, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn apply_appearance(&self, window: &mut Window, cx: &mut App) {
        match self.appearance {
            AppearanceMode::System => {
                cx.set_window_appearance(None);
                Theme::sync_system_appearance(Some(window), cx);
            }
            AppearanceMode::Light => {
                cx.set_window_appearance(Some(WindowAppearance::Light));
                Theme::change(ThemeMode::Light, Some(window), cx);
            }
            AppearanceMode::Dark => {
                cx.set_window_appearance(Some(WindowAppearance::Dark));
                Theme::change(ThemeMode::Dark, Some(window), cx);
            }
        }
    }

    pub fn appearance(&self) -> AppearanceMode {
        self.appearance
    }

    pub fn set_appearance_mode(
        &mut self,
        mode: AppearanceMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.appearance != mode {
            self.appearance = mode;
            self.apply_appearance(window, cx);
            self.persist_settings(cx);
            cx.notify();
        }
    }
}
