//! 外部文件夹的后台扫描；照片状态仍由 AppView 拥有。

use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::{AppContext as _, Context, PathPromptOptions, SharedString};

use super::super::{AppView, component::queue::is_supported_image};

#[derive(Default)]
pub(in crate::ui::app) struct PhotoImportState {
    pub pending: usize,
    pub revision: u64,
    pub feedback: Option<SharedString>,
}

#[derive(Default)]
struct ScannedPhotos {
    paths: Vec<PathBuf>,
    errors: Vec<String>,
}

/// 深度优先、按路径排序；规范化身份去重，也防止符号链接形成目录循环。
fn scan_photo_paths(paths: Vec<PathBuf>) -> ScannedPhotos {
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

impl AppView {
    pub(in crate::ui::app) fn pick_photo_folder(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("添加文件夹及子文件夹中的照片".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            this.update(cx, |this, cx| this.add_photos(paths, cx)).ok();
        })
        .detach();
    }

    pub(in crate::ui::app) fn import_photo_folders(
        &mut self,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.photo_import.pending += 1;
        self.photo_import.feedback = None;
        let revision = self.photo_import.revision;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let scanned = cx
                .background_spawn(async move { scan_photo_paths(paths) })
                .await;
            this.update(cx, |this, cx| {
                this.photo_import.pending -= 1;
                // 清空操作使之前的扫描失效，迟到的结果不能重新添加照片。
                if revision == this.photo_import.revision {
                    this.photo_import.feedback = if let Some(error) = scanned.errors.first() {
                        Some(format!("有 {} 处路径无法读取：{error}", scanned.errors.len()).into())
                    } else if scanned.paths.is_empty() {
                        Some("文件夹中没有支持的照片格式".into())
                    } else {
                        None
                    };
                    this.add_photos(scanned.paths, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "lumen-frame-import-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(path.join("nested/deeper")).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn recursively_collects_images_and_deduplicates_overlapping_paths() {
        let dir = Directory::new();
        let root_photo = dir.0.join("a.JPG");
        let nested_photo = dir.0.join("nested/deeper/b.png");
        for path in [&root_photo, &nested_photo, &dir.0.join("ignored.txt")] {
            std::fs::write(path, []).unwrap();
        }
        // 有图片扩展名的目录仍然需要递归，而不能被当成图片加入队列。
        std::fs::create_dir(dir.0.join("c.jpg")).unwrap();
        let result = scan_photo_paths(vec![
            dir.0.clone(),
            root_photo.clone(),
            dir.0.join("nested"),
        ]);
        assert_eq!(result.paths, [root_photo, nested_photo]);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn reports_unreadable_paths_and_keeps_empty_folders_out_of_the_queue() {
        let dir = Directory::new();
        let result = scan_photo_paths(vec![dir.0.clone(), dir.0.join("missing")]);
        assert!(result.paths.is_empty());
        assert_eq!(result.errors.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_cycles_do_not_duplicate_images_or_loop() {
        let dir = Directory::new();
        let photo = dir.0.join("a.jpg");
        std::fs::write(&photo, []).unwrap();
        std::os::unix::fs::symlink(&dir.0, dir.0.join("nested/back")).unwrap();
        let result = scan_photo_paths(vec![dir.0.clone()]);
        assert_eq!(result.paths, [photo]);
        assert!(result.errors.is_empty());
    }

    #[gpui_kit::test]
    fn clearing_while_scanning_discards_the_pending_import(cx: &mut gpui_kit::TestAppContext) {
        let dir = Directory::new();
        std::fs::write(dir.0.join("a.jpg"), []).unwrap();
        cx.update(gpui_kit::init);
        let (view, cx) =
            cx.add_window_view(|window, cx| AppView::new_with_settings_path(None, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.add_photos(vec![dir.0.clone()], cx);
            assert_eq!(view.photo_import.pending, 1);
            view.clear_photos(window, cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.photo_import.pending, 0);
            assert_eq!(view.photo_count(), 0);
            assert!(view.photo_import.feedback.is_none());
        });
    }
}
