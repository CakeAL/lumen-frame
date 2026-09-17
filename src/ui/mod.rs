//! 照片水印工作台的界面层。
//!
//! - [`page`] 组织可切换的完整页面；
//! - [`component`] 放置页面之间可复用的领域组件；
//! - [`behavior`] 放置不直接渲染页面的应用工作流；
//! - [`image`] 是图像管线与 GPUI 位图之间的专用适配层。

mod app;
pub mod image;

pub use app::{AppPage, AppView, ExportState, Thumbnail};
/// 保持预览任务的公共入口稳定，内部实现归入图像适配层。
pub use image::{
    PreviewJob, export_gainmap, render_gainmap_preview, render_preview, render_thumbnail,
};
