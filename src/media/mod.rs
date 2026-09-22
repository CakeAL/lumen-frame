//! 媒体文件与第三方解码库的基础设施适配层。

mod metadata;
mod vips;

pub use metadata::{ExifInfo, Rational};
pub use vips::{ensure_vips, load_base_image};
