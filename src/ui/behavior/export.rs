//! 照片批量导出流程。
//!
//! [`AppView`] 仍然拥有队列、参数与进度状态；本模块只把它们组织为一次可取消的串行导出，
//! 避免 libvips 的图像任务在界面层互相争抢资源。

use std::path::PathBuf;

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{Context, Window, prelude::*};

use crate::media::ExifInfo;
use crate::photo::Photo;

use super::super::{AppView, ExportState};

impl AppView {
    /// 按当前参数把队列里的照片全部导出。
    ///
    /// 逐张串行处理而不是并发：libvips 自己就吃满多核，再叠并发只会让每张都变慢，还会
    /// 让进度读数失去意义。
    pub(in crate::ui::app) fn export_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.is_empty() || matches!(self.export, ExportState::Running { .. }) {
            return;
        }
        if self.params.output_folder.is_none() {
            return;
        }

        let params = self.params.clone();
        let text_groups = self.build_text_groups(cx);
        let photos: Vec<(PathBuf, Option<ExifInfo>)> = self
            .workspace
            .photos()
            .iter()
            .map(|photo| (photo.path().to_path_buf(), photo.exif_override().cloned()))
            .collect();
        let total = photos.len();
        let window_handle = window.window_handle();

        self.export = ExportState::Running {
            completed: 0,
            total,
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut succeeded = 0usize;
            for (ix, (path, exif)) in photos.into_iter().enumerate() {
                let params = params.clone();
                let text_groups = text_groups.clone();
                let result = cx
                    .background_spawn(async move {
                        let mut photo = Photo::open_blocking(&path)?;
                        // 只有用户编辑过时才覆盖，未编辑的照片仍使用打开文件时读到的 EXIF。
                        if let Some(exif) = exif {
                            photo.exif = Some(exif);
                        }
                        let watermark = photo.generate_watermark(&params, &text_groups)?;
                        photo.save_image(&params, &watermark)
                    })
                    .await;

                if result.is_ok() {
                    succeeded += 1;
                }

                let completed = ix + 1;
                let alive = this
                    .update(cx, |this, cx| {
                        this.export = ExportState::Running { completed, total };
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }

            this.update(cx, |this, cx| {
                this.export = ExportState::Finished {
                    succeeded,
                    failed: total - succeeded,
                };
                if succeeded > 0 {
                    let message = format!("已成功导出 {succeeded} 张照片");
                    let _ = window_handle.update(cx, |_, window, cx| {
                        window.push_notification(Notification::success(message), cx);
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
