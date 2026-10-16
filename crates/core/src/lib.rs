//! 应用业务入口。UI 与未来的 MCP 适配器共用会话、命令及媒体用例。
//! 不接收 GPUI Context/Window/Entity，也不返回 GPU 位图。
pub mod export;
pub mod image;
pub mod import;
pub mod metadata;
mod storage;
pub mod text;
pub use storage::StoragePaths;
mod command;
pub use command::{Command, CommandOutput, ErrorCode, OperationError, Reply, Request};
mod session;
pub use session::Session;
// 持久化和独立工作流通过同一后端命名空间公开，UI 不再承载这些实现。
pub use crate::features::{colour_gainmap, geolocation, motion_photo, photo_palette};
pub use crate::persistence::{logos, presets, settings};

/// 无窗口宿主在主线程初始化原生字体服务；桌面宿主仍注入与窗口共用的字体系统。
pub fn initialize_native_text() {
    crate::render::init_font_backend(gpui_kit::platform::current_platform(true).text_system());
}

pub mod features;
pub mod gainmap;
pub mod media;
pub mod persistence;
pub mod photo;
pub mod render;
pub mod rotation;
pub mod update;
pub mod watermark;
pub mod workspace;
