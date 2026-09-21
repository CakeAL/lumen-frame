use std::env;

fn main() {
    const WINDOWS_ICON: &str = "assets/app-icon/LumenFrame.ico";

    println!("cargo:rerun-if-changed={WINDOWS_ICON}");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(WINDOWS_ICON);
    resource
        .compile()
        .expect("无法把 LumenFrame.ico 嵌入 Windows 可执行文件");
}
