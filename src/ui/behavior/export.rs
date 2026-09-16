//! 照片批量导出流程。
//!
//! [`AppView`] 仍然拥有队列、参数与进度状态；本模块只把它们组织为一次可取消的串行导出，
//! 避免 libvips 的图像任务在界面层互相争抢资源。

use std::path::PathBuf;

use gpui_kit::{Context, prelude::*};

use crate::photo::Photo;

use super::super::{AppView, ExportState};

impl AppView {
    /// 按当前参数把队列里的照片全部导出。
    ///
    /// 逐张串行处理而不是并发：libvips 自己就吃满多核，再叠并发只会让每张都变慢，还会
    /// 让进度读数失去意义。
    pub(in crate::ui::app) fn export_all(&mut self, cx: &mut Context<Self>) {
        if self.workspace.is_empty() || matches!(self.export, ExportState::Running { .. }) {
            return;
        }
        if self.params.output_folder.is_none() {
            return;
        }

        let params = self.params.clone();
        let text_groups = self.build_text_groups(cx);
        let paths: Vec<PathBuf> = self
            .workspace
            .photos()
            .iter()
            .map(|photo| photo.path().to_path_buf())
            .collect();
        let total = paths.len();

        self.export = ExportState::Running {
            completed: 0,
            total,
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut succeeded = 0usize;
            for (ix, path) in paths.into_iter().enumerate() {
                let params = params.clone();
                let text_groups = text_groups.clone();
                let result = cx
                    .background_spawn(async move {
                        let photo = Photo::open_blocking(&path)?;
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
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
