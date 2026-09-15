//! 顶部页面标签。

use gpui_kit::Context;
use gpui_kit::component::{
    Icon, IconName, Sizable as _,
    button::Button,
    h_flex,
    tab::{Tab, TabBar},
};
use gpui_kit::prelude::*;

use super::{AppPage, AppView};

impl AppView {
    pub(super) fn render_page_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .h_full()
            .items_center()
            .pr_3()
            .child(
                TabBar::new("app-page-tabs")
                    .pill()
                    .small()
                    .flex_1()
                    .min_w_0()
                    .when(self.page == AppPage::Watermark, |this| {
                        this.selected_index(0)
                    })
                    .child(
                        Tab::new()
                            .prefix(Icon::new(IconName::Frame).left_2())
                            .label("边框水印"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.go_to(AppPage::Watermark, cx))),
            )
            .child(
                Button::new("app-settings")
                    .icon(IconName::Settings)
                    .label("设置")
                    .outline()
                    .small()
                    .tooltip("设置")
                    .accessibility_label("设置")
                    .on_click(cx.listener(|this, _, _, cx| this.go_to(AppPage::Settings, cx))),
            )
    }
}
