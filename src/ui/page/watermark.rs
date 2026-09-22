//! 应用窗口框架与页面布局。

use gpui_kit::component::{ActiveTheme as _, Root, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, ExternalPaths, Window};

use super::super::{AppPage, AppView};

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(self.render_page_body(cx))
            // 叠加层必须由应用的第一个视图渲染一次，`Root` 只负责协调它们。
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

impl AppView {
    /// 页面主体只负责在页面间切换；水印页自己的三栏工作区由专用方法组合。
    fn render_page_body(&self, cx: &Context<Self>) -> AnyElement {
        match self.page {
            AppPage::Watermark => self.render_watermark_workspace(cx).into_any_element(),
            AppPage::GainMap => self.render_gainmap_page(cx).into_any_element(),
            AppPage::OtherTools => self.render_other_tools_page(cx).into_any_element(),
            AppPage::Settings => self.render_settings_page(cx).into_any_element(),
        }
    }

    /// 水印工作区：预设面板、照片工作区与参数检查器三栏。
    fn render_watermark_workspace(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .child(self.render_preset_panel(cx))
            .child(self.render_photo_workspace(cx))
            .child(self.render_inspector(cx))
    }

    /// 照片工作区：上方预览、下方队列。
    fn render_photo_workspace(&self, cx: &Context<Self>) -> impl IntoElement {
        // 拖放挂在整块工作区上：拖到预览上也算数，不必瞄准下面那条队列。
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_preview_pane(cx))
                    .child(self.render_queue_pane(cx)),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_photos(paths.paths().to_vec(), cx)
            }))
    }
}
