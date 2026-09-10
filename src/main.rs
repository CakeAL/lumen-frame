//! Lumen Frame：为照片加上边框、阴影与 EXIF 文字水印。

use gpui_kit::component::*;
use gpui_kit::*;

use lumen_frame::ui::AppView;

/// 让 libvips 到应用包里找它的格式模块。
///
/// libvips 把 HEIC/AVIF、JXL 这类可选格式的加载器做成了运行期 `g_module_open` 的模块，
/// 查找路径来自编译期写死的 `libdir`——而 Homebrew 的 bottle 里那个路径是**构建机**的
/// （`/Users/runner/work/sharp-libvips/...`），在谁的机器上都不存在。打包时模块被放进
/// `<App>.app/Contents/lib/vips-modules-8.18/`，这里在 `vips_init` 之前告诉它去哪找。
///
/// 没打包时（`cargo run`）这个目录不存在，函数什么都不做，交给 libvips 自己的逻辑。
fn point_vips_at_bundled_modules() {
    if std::env::var_os("VIPS_LIBDIR").is_some() {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(exe_dir) = executable.parent() else {
        return;
    };

    // 两种打包布局：
    //   macOS —— <App>.app/Contents/MacOS/lumen-frame + Contents/lib/vips-modules-8.18
    //   Windows —— lumen-frame.exe + vips-modules-8.18 与 DLL 并排
    let candidates = [
        exe_dir.join("lib"),
        exe_dir.parent().map(|contents| contents.join("lib")).unwrap_or_default(),
        exe_dir.to_path_buf(),
    ];

    let Some(lib_dir) = candidates
        .into_iter()
        .find(|dir| dir.join("vips-modules-8.18").is_dir())
    else {
        return;
    };
    // SAFETY: 在 main 的最开头、任何线程启动之前设置环境变量。
    unsafe { std::env::set_var("VIPS_LIBDIR", &lib_dir) };
}

fn main() {
    point_vips_at_bundled_modules();

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
