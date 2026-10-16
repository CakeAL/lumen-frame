/// 原生平台须在主线程创建；这里只在测试入口取得字体服务，实际渲染仍由测试线程执行。
#[ctor::ctor(unsafe)]
fn init_fonts() {
    lumen_frame::render::init_font_backend(
        gpui_kit::platform::current_platform(true).text_system(),
    );
}
