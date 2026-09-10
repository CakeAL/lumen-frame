//! Lumen Frame：为照片加上边框、阴影与 EXIF 文字水印。

use gpui_kit::component::*;
use gpui_kit::*;

use lumen_frame::ui::AppView;

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        // 组件内置文案（取色器、日历等）跟随应用语言。
        gpui_kit::component::set_locale("zh-CN");

        cx.spawn(async move |cx| {
            cx.open_window(
                WindowOptions {
                    // 三栏工作区在更窄的窗口里会挤掉预览，所以给一个明确的窗口下限。
                    window_min_size: Some(size(px(1040.), px(680.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Lumen Frame".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| AppView::new(window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Failed to open window");
        })
        .detach();
    });
}
