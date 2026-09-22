//! libvips 的进程生命周期与基础图片解码。

use std::{path::Path, sync::OnceLock};

use anyhow::{Context as _, Result};
use libvips::{VipsApp, VipsImage, ops};

fn vips() -> &'static VipsApp {
    static VIPS: OnceLock<VipsApp> = OnceLock::new();
    VIPS.get_or_init(|| VipsApp::default("lumen-frame").expect("failed to init libvips"))
}

/// 保证 libvips 已初始化，并让进程级实例存活到程序退出。
pub fn ensure_vips() -> &'static VipsApp {
    vips()
}

/// 加载图片并按 EXIF orientation 摆正，同时保留 Ultra HDR gain map 元数据。
pub fn load_base_image(path: &Path) -> Result<VipsImage> {
    ensure_vips();
    let image =
        VipsImage::new_from_file(&path.to_string_lossy()).context("failed to load image")?;
    ops::autorot(&image).context("failed to autorotate image")
}
