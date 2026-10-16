//! 文件发现和支持格式；不依赖窗口或拖放事件。
use std::{collections::HashSet, path::PathBuf};
/// 队列接受的图片扩展名。
///
/// 不直接读 libvips 支持的格式列表：这里同时是给用户的提示——拖入非图片文件时会被
/// 安静地忽略，而不是排进队列后在预览里报错。
// macOS 随包使用完整的 Homebrew libvips；Windows web 包已验证 AVIF，HEIC 和相机 RAW 尚未支持。
#[cfg(target_os = "macos")]
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "tif", "tiff", "heic", "heif", "avif", "bmp", "gif", "raf",
    "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "pef", "srw",
];
#[cfg(not(target_os = "macos"))]
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "tif", "tiff", "avif", "bmp", "gif",
];

pub fn is_supported_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| SUPPORTED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Default)]
pub struct ScannedPhotos {
    pub paths: Vec<PathBuf>,
    pub errors: Vec<String>,
}

/// 深度优先、按路径排序；规范化身份去重，也防止符号链接形成目录循环。
pub fn scan_photo_paths(paths: Vec<PathBuf>) -> ScannedPhotos {
    let mut result = ScannedPhotos::default();
    let mut pending: Vec<_> = paths.into_iter().rev().collect();
    let mut visited = HashSet::new();
    while let Some(path) = pending.pop() {
        let identity = match path.canonicalize() {
            Ok(identity) => identity,
            Err(error) => {
                result.errors.push(format!("{}：{error}", path.display()));
                continue;
            }
        };
        if !visited.insert(identity) {
            continue;
        }
        if path.is_dir() {
            let entries = match std::fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) => {
                    result.errors.push(format!("{}：{error}", path.display()));
                    continue;
                }
            };
            let mut children = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => children.push(entry.path()),
                    Err(error) => result.errors.push(format!("{}：{error}", path.display())),
                }
            }
            children.sort();
            pending.extend(children.into_iter().rev());
        } else if path.is_file() && is_supported_image(&path) {
            result.paths.push(path);
        }
    }
    result
}
