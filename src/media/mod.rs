//! 媒体文件与第三方解码库的基础设施适配层。

mod metadata;
pub(crate) mod vips;

pub use metadata::{ExifInfo, Rational};
pub use vips::{ensure_vips, image_metadata_string, image_write_to_memory, load_base_image};
