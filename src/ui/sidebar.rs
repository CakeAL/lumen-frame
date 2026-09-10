//! 左侧导航。
//!
//! 侧栏只承担一件事：在「照片水印」和「设置」之间切换。设置固定在底部，这样主功能的
//! 位置永远不变，不用先找到它再点进去。

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, h_flex,
    sidebar::{Sidebar, SidebarItem as _, SidebarMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, Window, div};

use super::{AppPage, AppView};

impl AppView {
    pub(super) fn render_sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings_entry = SidebarMenuItem::new("设置")
            .icon(Icon::new(IconName::Settings))
            .active(self.page == AppPage::Settings)
            .on_click(cx.listener(|this, _, _, cx| this.go_to(AppPage::Settings, cx)));

        Sidebar::new("app-sidebar")
            .collapsible(false)
            .header(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::Frame).text_color(cx.theme().primary))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().sidebar_foreground)
                            .child("Lumen Frame"),
                    ),
            )
            .child(
                SidebarMenuItem::new("照片水印")
                    .icon(Icon::new(IconName::Frame))
                    .active(self.page == AppPage::Watermark)
                    .on_click(cx.listener(|this, _, _, cx| this.go_to(AppPage::Watermark, cx))),
            )
            // 底部条目走和导航条目完全相同的渲染路径，几何、悬停和选中态因此不会漂移。
            .footer(settings_entry.render("sidebar-settings", window, cx))
    }
}
