//! 可复用的本机 Logo 素材页。模板引用稳定编号，照片和预设不复制本机文件路径。

use super::super::{AppView, component::field::description};
use crate::persistence::logos::{self, LogoAsset};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, button::Button, h_flex,
    scroll::ScrollableElement as _, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    ClipboardItem, Context, ExternalPaths, ObjectFit, PathPromptOptions, RenderImage, SharedString,
    div, img,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

pub(in crate::ui::app) struct LogoPageState {
    pub directory: Option<PathBuf>,
    pub assets: Vec<LogoAsset>,
    thumbnails: HashMap<u64, Arc<RenderImage>>,
    busy: bool,
    feedback: Option<SharedString>,
}

impl LogoPageState {
    pub fn new() -> Self {
        let directory = logos::directory();
        let loaded = directory.as_deref().map(logos::load_in).transpose();
        let (assets, feedback) = match loaded {
            Ok(assets) => (assets.unwrap_or_default(), None),
            Err(error) => (
                Vec::new(),
                Some(format!("无法载入 Logo 素材库：{error:#}").into()),
            ),
        };
        Self {
            directory,
            assets,
            feedback,
            thumbnails: HashMap::new(),
            busy: false,
        }
    }
}

impl AppView {
    pub(in crate::ui::app) fn load_logo_thumbnails(&mut self, cx: &mut Context<Self>) {
        let assets = self.logos.assets.clone();
        cx.spawn(async move |this, cx| {
            let thumbnails = cx
                .background_spawn(async move {
                    assets
                        .iter()
                        .map(|asset| {
                            (
                                asset.id(),
                                super::super::super::image::render_logo_thumbnail(asset.bytes()),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                for (id, result) in thumbnails {
                    match result {
                        Ok(image) => {
                            this.logos.thumbnails.insert(id, image);
                        }
                        Err(error) => {
                            this.logos.feedback =
                                Some(format!("自定义logo{id} 无法预览：{error:#}").into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn pick_custom_logos(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("选择 Logo（PNG、SVG、JPEG、WebP）".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                this.update(cx, |this, cx| this.import_custom_logos(paths, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn import_custom_logos(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.logos.busy || paths.is_empty() {
            return;
        }
        let Some(dir) = self.logos.directory.clone() else {
            self.logos.feedback = Some("找不到系统的素材保存目录".into());
            cx.notify();
            return;
        };
        self.logos.busy = true;
        self.logos.feedback = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { logos::import_in(&dir, &paths) })
                .await;
            this.update(cx, |this, cx| {
                this.logos.busy = false;
                match result {
                    Ok(assets) => {
                        this.params.custom_logos = logos::snapshot(&assets);
                        this.logos.assets = assets;
                        this.load_logo_thumbnails(cx);
                        this.refresh_preview(cx);
                    }
                    Err(error) => {
                        this.logos.feedback = Some(format!("导入 Logo 失败：{error:#}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::ui::app) fn render_logos_page(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex().id("custom-logos-page").flex_1().min_h_0().gap_4().p_6()
            .child(h_flex().w_full().justify_between().gap_4()
                .child(v_flex().gap_2().child(div().text_lg().font_weight(gpui_kit::FontWeight::SEMIBOLD).child("自定义 Logo"))
                    .child(description("在文字模板中输入 {自定义logo1}。调节该行字号即可缩放，透明度与原始颜色会保留。")))
                .child(h_flex().gap_2()
                    .child(Button::new("logo-open-folder").label("打开素材文件夹").outline().disabled(self.logos.directory.is_none() || self.logos.assets.is_empty()).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(dir) = &this.logos.directory { cx.open_with_system(dir); }
                    })))
                    .child(Button::new("logo-import").icon(IconName::Plus).label("导入 Logo…").outline().loading(self.logos.busy).disabled(self.logos.busy).on_click(cx.listener(|this, _, _, cx| this.pick_custom_logos(cx))))))
            .when_some(self.logos.feedback.clone(), |this, feedback| this.child(description(feedback)))
            .child(v_flex().id("custom-logos-scroll").flex_1().min_h_0().gap_3().overflow_y_scrollbar()
                .when(self.logos.assets.is_empty(), |this| this.child(description("还没有自定义 Logo。导入文件或将文件拖到这里。")))
                .children(self.logos.assets.iter().map(|asset| {
                    let token = asset.token();
                    let copy_token = token.clone();
                    h_flex().id(("custom-logo", asset.id())).items_center().gap_4().p_4().border_1().border_color(cx.theme().border).rounded(cx.theme().radius)
                        .child(div().w_24().h_20().flex_shrink_0().bg(cx.theme().muted).rounded(cx.theme().radius)
                            .when_some(self.logos.thumbnails.get(&asset.id()).cloned(), |this, image| this.child(img(image).size_full().object_fit(ObjectFit::Contain))))
                        .child(v_flex().flex_1().min_w_0().gap_1().child(asset.name()).child(description(token)))
                        .child(Button::new(("copy-logo-token", asset.id())).label("复制字段").outline().small().on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy_token.clone()))))
                })))
            .child(description(self.logos.directory.as_ref().map(|path| format!("素材保存在 {}", path.display())).unwrap_or_default()))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.import_custom_logos(paths.paths().to_vec(), cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, component::Root, test::TestWindowExt};
    use std::{cell::RefCell, rc::Rc};
    #[gpui_kit::test]
    fn custom_logo_page_imports_and_copies_template_fields(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let slot = slot.clone();
            move |window, cx| {
                let app = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                slot.borrow_mut().replace(app.clone());
                Root::new(app, window, cx)
            }
        });
        let app = slot.borrow().clone().unwrap();
        let root = std::env::temp_dir().join(format!("lumen-frame-logo-ui-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.svg");
        std::fs::write(&source, br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="#ffca00"/></svg>"##).unwrap();
        app.update_in(cx, |app, _, cx| {
            app.logos.directory = Some(root.join("support"));
            app.logos.assets.clear();
            app.logos.thumbnails.clear();
            app.go_to(super::super::super::AppPage::CustomLogos, cx);
            app.import_custom_logos(vec![source], cx);
        });
        cx.run_until_parked();
        assert_eq!(app.read_with(cx, |app, _| app.logos.assets.len()), 1);
        cx.update(|window, cx| window.click(("copy-logo-token", 1_u64), cx));
        assert_eq!(
            cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
            "{自定义logo1}"
        );
        app.update_in(cx, |app, window, cx| {
            assert!(app.params.custom_logos.contains_key("自定义logo1"));
            app.restore_watermark(
                crate::persistence::presets::load_builtin("基础样式").unwrap(),
                window,
                cx,
            );
            assert!(app.params.custom_logos.contains_key("自定义logo1"));
        });
        cx.run_until_parked();
        std::fs::remove_dir_all(root).unwrap();
    }
}
