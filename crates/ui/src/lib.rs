//! GPUI 界面与交互；业务模型和媒体操作由 lumen-frame-core 提供。
use lumen_frame_core as backend;
mod app;
pub mod image;
pub mod theme;
mod window_effects;
pub use app::{AppPage, AppView, ExportState, Thumbnail};
pub use image::{
    PreviewJob, export_colour_gainmap, export_gainmap, render_colour_gainmap_preview,
    render_gainmap_preview, render_preview, render_thumbnail,
};

#[cfg(test)]
#[ctor::ctor(unsafe)]
fn initialize_test_fonts() {
    backend::initialize_native_text();
}
