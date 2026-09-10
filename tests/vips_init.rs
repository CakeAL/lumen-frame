//! libvips 的初始化契约。
//!
//! 这个测试单独占一个文件，因为它必须在**一个全新的进程**里成为**第一次**触碰 libvips
//! 的代码。libvips 的操作类哈希是首次调用时惰性构建的（`vips_operation_new` → GLib 的
//! `g_once`），一旦在 `vips_init` 之前调用、或者被多个线程同时首次调用，就会崩在
//! `vips_class_map_all` 里。
//!
//! 真实故障现场：队列缩略图和预览合成并发跑在 GPUI 的后台线程池上，缩略图那条路径没有
//! 先初始化 libvips，于是段错误落在 `preview_image::load_scaled`。

use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use lumen_frame::ui::render_thumbnail;

const PHOTO: &str = "./test_images/DSC_4587.jpg";

/// 多个线程同时第一次使用 libvips 时，必须全部成功。
#[test]
fn concurrent_first_use_of_vips_is_safe() {
    const THREADS: usize = 8;

    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let barrier = barrier.clone();
            thread::spawn(move || {
                // 让所有线程尽可能同时进入第一次 vips 调用。
                barrier.wait();
                render_thumbnail(Path::new(PHOTO))
            })
        })
        .collect();

    for (ix, handle) in handles.into_iter().enumerate() {
        let image = handle
            .join()
            .unwrap_or_else(|_| panic!("第 {ix} 个线程在 vips 调用里 panic"))
            .unwrap_or_else(|error| panic!("第 {ix} 个线程缩略图失败：{error:#}"));
        assert!(image.size(0).width.0 > 0);
    }
}
