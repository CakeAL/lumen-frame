#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! Lumen Frame：为照片加上边框、阴影与 EXIF 文字水印。

mod options;

use std::borrow::Cow;

use gpui_kit::component::*;
use gpui_kit::*;

use lumen_frame_ui::AppView;

#[derive(Clone, Copy)]
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "qr-code/wechat-reward-code.jpg" => {
                Some(include_bytes!("../assets/qr_code/wechat_reward_code.jpg"))
            }
            "qr-code/rednote.jpg" => Some(include_bytes!("../assets/qr_code/rednote.jpg")),
            "qr-code/bilibili.jpg" => Some(include_bytes!("../assets/qr_code/bilibili.jpg")),
            _ => None,
        };
        if let Some(bytes) = bytes {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::AllAssets.list(path)?;
        paths.extend(
            [
                "qr-code/wechat-reward-code.jpg",
                "qr-code/rednote.jpg",
                "qr-code/bilibili.jpg",
            ]
            .into_iter()
            .filter(|asset| asset.starts_with(path))
            .map(SharedString::from),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

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
        exe_dir
            .parent()
            .map(|contents| contents.join("lib"))
            .unwrap_or_default(),
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

/// 主窗口拥有应用的窗口生命周期；子窗口持有 AppView，不能依赖它被释放来收尾。
fn close_auxiliary_windows_with_main(main_window: AnyWindowHandle, cx: &App) {
    cx.on_window_closed(move |cx, closed| {
        if closed == main_window.window_id() {
            // 等当前关闭通知完成后再关闭其它窗口，避免嵌套触发同一组关闭回调。
            cx.defer(|cx| {
                for handle in cx.windows() {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            });
        }
    })
    .detach();
}

fn main() -> anyhow::Result<()> {
    point_vips_at_bundled_modules();
    let options = options::Options::parse(std::env::args_os().skip(1))?;
    if options.help {
        options::print_help();
        return Ok(());
    }
    if options.mcp_stdio {
        lumen_frame_core::initialize_native_text();
        let storage = options
            .mcp_storage
            .map(lumen_frame_core::StoragePaths::in_directory)
            .unwrap_or_default();
        let session = lumen_frame_core::Session::from_storage(storage)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        return runtime.block_on(lumen_frame_mcp::serve_stdio(session));
    }
    let platform = gpui_kit::platform::current_platform(false);
    lumen_frame_core::render::init_font_backend(platform.text_system());
    let app = gpui_kit::Application::with_platform(platform)
        .with_assets(AppAssets)
        // 主窗口关闭时会统一收起子窗口，最后一个窗口关闭后退出应用。
        .with_quit_mode(QuitMode::LastWindowClosed);

    app.run(move |cx| {
        gpui_kit::init(cx);
        // 组件内置文案（取色器、日历等）跟随应用语言。
        gpui_kit::component::set_locale("zh-CN");

        let (main_window, _) = gpui_kit::open_window(
            WindowOptions {
                // 预设、工作区与参数面板都需要保留可用宽度。
                window_min_size: Some(size(px(1180.), px(680.))),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| cx.new(|cx| AppView::new(window, cx)),
        )
        .expect("无法打开主窗口");
        close_auxiliary_windows_with_main(main_window, cx);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, close_auxiliary_windows_with_main};
    use gpui_kit::AssetSource as _;

    #[test]
    fn qr_codes_are_embedded_in_the_application_assets() {
        for path in [
            "qr-code/wechat-reward-code.jpg",
            "qr-code/rednote.jpg",
            "qr-code/bilibili.jpg",
        ] {
            let bytes = AppAssets
                .load(path)
                .unwrap_or_else(|error| panic!("读取 {path} 失败：{error:#}"))
                .unwrap_or_else(|| panic!("缺少内置素材：{path}"));
            image::load_from_memory(&bytes)
                .unwrap_or_else(|error| panic!("{path} 不是有效图片：{error}"));
        }
    }

    #[gpui_kit::test]
    fn main_window_closes_auxiliary_windows(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let main = cx.add_empty_window().update(|window, cx| {
            let handle = window.window_handle();
            close_auxiliary_windows_with_main(handle, cx);
            handle
        });
        let child = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        cx.add_empty_window();
        cx.update(|cx| {
            child
                .update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(cx.windows().len(), 2, "单独关闭子窗口不应关闭主窗口"));
        cx.add_empty_window();
        cx.update(|cx| {
            main.update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        cx.run_until_parked();
        cx.read(|cx| assert!(cx.windows().is_empty(), "主窗口关闭后不得残留子窗口"));
    }
}
