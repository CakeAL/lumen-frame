//! 应用外观偏好：主题槽位、明暗模式、界面缩放与照片展示背景。

use std::path::PathBuf;

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Context, SharedString, Window, WindowAppearance};

use crate::backend::settings::{self as settings_store, AppSettings, AppearanceMode};
use crate::backend::watermark::{DEFAULT_FONT, WatermarkParams};

use super::super::{AppView, page::settings};

/// 应用本机偏好的唯一 UI 持有者，避免把 `AppSettings` 拆散到 `AppView` 上手动双向同步。
pub(in crate::app) struct PreferencesState {
    pub values: AppSettings,
    pub path: Option<PathBuf>,
    pub feedback: Option<SharedString>,
}

impl PreferencesState {
    pub fn new(path: Option<PathBuf>, mut values: AppSettings) -> Self {
        values.interface_scale = Some(
            values
                .interface_scale
                .unwrap_or(settings::DEFAULT_INTERFACE_SCALE),
        );
        values.preview_max_edge = Some(settings::preview_max_edge_from_settings(
            values.preview_max_edge,
        ));
        Self {
            values,
            path,
            feedback: None,
        }
    }

    pub fn interface_scale(&self) -> f32 {
        self.values
            .interface_scale
            .unwrap_or(settings::DEFAULT_INTERFACE_SCALE)
    }

    pub fn preview_max_edge(&self) -> i32 {
        settings::preview_max_edge_from_settings(self.values.preview_max_edge)
    }

    pub fn apply_saved(&mut self, saved: &AppSettings) {
        let mut next = saved.clone();
        next.interface_scale = Some(
            saved
                .interface_scale
                .unwrap_or(settings::DEFAULT_INTERFACE_SCALE),
        );
        next.preview_max_edge = Some(settings::preview_max_edge_from_settings(
            saved.preview_max_edge,
        ));
        self.values = next;
    }

    pub fn reset_appearance(&mut self) {
        self.values.appearance = AppearanceMode::System;
        self.values.window_effects = true;
        self.values.light_theme = None;
        self.values.dark_theme = None;
        self.values.interface_scale = Some(settings::DEFAULT_INTERFACE_SCALE);
        self.values.preview_max_edge = Some(settings::DEFAULT_PREVIEW_MAX_EDGE);
        self.values.preview_background = settings_store::DEFAULT_PREVIEW_BACKGROUND;
    }

    pub fn save(&mut self, params: &WatermarkParams) {
        let mut snapshot = self.values.clone();
        snapshot.interface_scale = Some(self.interface_scale());
        snapshot.preview_max_edge = Some(self.preview_max_edge());
        params.write_to_settings(&mut snapshot);
        let Some(path) = self.path.as_deref() else {
            self.feedback = Some("找不到系统的配置目录，偏好未能保存。".into());
            return;
        };
        self.feedback = settings_store::save_at(path, &snapshot)
            .err()
            .map(|error| format!("偏好没能保存：{error:#}").into());
    }
}

impl AppView {
    pub(in crate::app) fn set_window_effects(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.preferences.values.window_effects == enabled {
            return;
        }
        self.preferences.values.window_effects = enabled;
        crate::window_effects::set_enabled(enabled, cx);
        self.persist_settings();
        cx.notify();
    }

    pub fn interface_scale(&self) -> f32 {
        self.preferences.interface_scale()
    }

    pub fn preview_max_edge(&self) -> i32 {
        self.preferences.preview_max_edge()
    }

    pub fn light_theme_name(&self) -> Option<String> {
        self.preferences.values.light_theme.clone()
    }

    pub fn dark_theme_name(&self) -> Option<String> {
        self.preferences.values.dark_theme.clone()
    }

    pub fn set_interface_scale(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences.values.interface_scale = Some(scale);
        settings::apply_interface_scale(scale, window, cx);
        self.persist_settings();
        cx.notify();
    }

    pub(in crate::app) fn set_preview_max_edge(&mut self, max_edge: i32) {
        if self.preferences.preview_max_edge() == max_edge {
            return;
        }
        self.preferences.values.preview_max_edge = Some(max_edge);
        self.persist_settings();
    }

    pub(in crate::app) fn set_preview_background(&mut self, rgb: [u8; 3]) {
        if self.preferences.values.preview_background != rgb {
            self.preferences.values.preview_background = rgb;
            self.persist_settings();
        }
    }

    pub(in crate::app) fn set_default_font(&mut self, font: String, cx: &mut Context<Self>) {
        if self.backend.params().default_font == font {
            return;
        }
        self.backend.params_mut().default_font = font;
        self.persist_settings();
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(in crate::app) fn apply_theme_slots(&self, cx: &mut App) {
        let configs: Vec<_> = [
            &self.preferences.values.light_theme,
            &self.preferences.values.dark_theme,
        ]
        .into_iter()
        .flatten()
        .filter_map(|name| crate::theme::find(name, cx))
        .collect();
        Theme::update(cx, |theme| {
            for config in configs {
                theme.apply_config(&config);
            }
        });
    }

    pub(in crate::app) fn persist_settings(&mut self) {
        self.backend.mark_settings_changed();
        let params = self.backend.params().clone();
        self.preferences.save(&params);
    }

    pub fn reset_appearance_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences.reset_appearance();
        crate::window_effects::set_enabled(true, cx);
        self.backend.params_mut().default_font = DEFAULT_FONT.to_owned();
        let (light, dark) = {
            let registry = ThemeRegistry::global(cx);
            (
                registry.default_light_theme().clone(),
                registry.default_dark_theme().clone(),
            )
        };
        Theme::update(cx, |theme| {
            theme.apply_config(&light);
            theme.apply_config(&dark);
        });
        let (light_name, dark_name) = (light.name.clone(), dark.name.clone());
        self.settings.light_theme.update(cx, |state, cx| {
            state.set_selected_value(&light_name, window, cx)
        });
        self.settings.dark_theme.update(cx, |state, cx| {
            state.set_selected_value(&dark_name, window, cx)
        });
        let default_font = SharedString::from(self.backend.params().default_font.clone());
        self.settings.default_font.update(cx, |state, cx| {
            state.set_selected_value(&default_font, window, cx)
        });
        self.settings.preview_background.sync(
            self.preferences.values.preview_background,
            window,
            cx,
        );
        self.settings.preview_max_edge.sync(
            f64::from(self.preferences.preview_max_edge()),
            window,
            cx,
        );
        settings::apply_interface_scale(self.preferences.interface_scale(), window, cx);
        self.apply_appearance(window, cx);
        self.persist_settings();
        self.preferences.feedback = Some("已恢复默认设置。".into());
        self.refresh_preview(cx);
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
            ThemeMode::Light => self.preferences.values.light_theme = Some(name.to_string()),
            ThemeMode::Dark => self.preferences.values.dark_theme = Some(name.to_string()),
        }
        self.apply_theme_slots(cx);
        self.apply_appearance(window, cx);
        self.persist_settings();
        cx.notify();
    }

    pub(in crate::app) fn apply_appearance(&self, window: &mut Window, cx: &mut App) {
        match self.preferences.values.appearance {
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
        self.preferences.values.appearance
    }

    pub fn set_appearance_mode(
        &mut self,
        mode: AppearanceMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.values.appearance != mode {
            self.preferences.values.appearance = mode;
            self.apply_appearance(window, cx);
            self.persist_settings();
            cx.notify();
        }
    }
}
