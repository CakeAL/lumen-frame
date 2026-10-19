//! 窗口背景的原生效果与主题遮罩；不改变控件、照片预览或导出颜色。

use gpui_kit::base::{Root, RootPlugin};
use gpui_kit::component::{ActiveTheme as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Div, Global, IntoElement, Render, Stateful, Styled as _, Window,
    WindowBackgroundAppearance, div,
};

/// 持久化偏好在窗口层的投影，供主窗口及后来打开的编辑窗口共用。
struct WindowEffects(bool);
impl Global for WindowEffects {}

pub(crate) const fn supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

pub(crate) fn enabled(cx: &App) -> bool {
    supported()
        && cx
            .try_global::<WindowEffects>()
            .is_some_and(|state| state.0)
}

pub(crate) fn install(value: bool, cx: &mut App) {
    cx.set_global(WindowEffects(value));
    // 在 Component 的 Root 插件后覆盖底层背景，保留其弹层、焦点和窗口边框。
    Root::register_plugin(cx, |_, _| WindowBackdrop { applied: None });
}

pub(crate) fn set_enabled(value: bool, cx: &mut App) {
    cx.set_global(WindowEffects(value));
    cx.refresh_windows();
}

pub(crate) fn title_bar(cx: &App) -> TitleBar {
    TitleBar::new().when(enabled(cx), |bar| bar.bg(cx.theme().transparent))
}

struct WindowBackdrop {
    applied: Option<bool>,
}

impl RootPlugin for WindowBackdrop {
    fn prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let enabled = enabled(cx);
        if self.applied == Some(enabled) {
            return;
        }
        let appearance = if enabled {
            if cfg!(target_os = "windows") {
                WindowBackgroundAppearance::MicaBackdrop
            } else {
                WindowBackgroundAppearance::Blurred
            }
        } else {
            WindowBackgroundAppearance::Opaque
        };
        window.set_background_appearance(appearance);
        self.applied = Some(enabled);
    }

    fn style(&self, surface: &mut Stateful<Div>, _: &mut Window, cx: &mut App) {
        // 只画一层主题遮罩。各页面继承此背景，避免多层半透明背景叠成不透明。
        let background = cx
            .theme()
            .background
            .alpha(if enabled(cx) { 0.8 } else { 1.0 });
        surface.style().background = Some(background.into());
    }
}

impl Render for WindowBackdrop {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
