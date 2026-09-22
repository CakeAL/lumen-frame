//! 主窗口标题栏与页面导航。

use gpui_kit::Context;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Sizable as _, TitleBar,
    button::Button,
    h_flex,
    tab::{Tab, TabBar},
};
use gpui_kit::prelude::*;

use super::super::{AppPage, AppView};

impl AppView {
    /// 主窗口的原生标题栏承载应用级导航；内容工作区不再拥有第二套侧栏导航。
    pub(in crate::ui::app) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(self.render_title_bar_navigation(cx))
    }

    fn render_title_bar_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .when(self.page == AppPage::GainMap, |this| this.selected_index(1))
                    .when(self.page == AppPage::OtherTools, |this| {
                        this.selected_index(2)
                    })
                    .child(
                        Tab::new()
                            .prefix(Icon::new(IconName::Frame).left_2())
                            .label("边框水印"),
                    )
                    .child(
                        Tab::new()
                            .prefix(Icon::new(IconName::Images).left_2())
                            .label("预览Gainmap"),
                    )
                    .child(
                        Tab::new()
                            .prefix(Icon::new(IconName::Palette).left_2())
                            .label("小工具"),
                    )
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        let page = match index {
                            0 => AppPage::Watermark,
                            1 => AppPage::GainMap,
                            2 => AppPage::OtherTools,
                            _ => return,
                        };
                        this.go_to(page, cx);
                    })),
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
