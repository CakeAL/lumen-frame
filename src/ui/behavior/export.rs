//! 照片批量导出流程。
//!
//! [`AppView`] 仍然拥有队列、参数与进度状态；本模块只把它们组织为一次可取消的串行导出，
//! 避免 libvips 的图像任务在界面层互相争抢资源。

use std::path::PathBuf;

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{Context, Window, prelude::*};

use crate::media::ExifInfo;
use crate::persistence::presets::WatermarkPreset;
use crate::photo::Photo;

use super::super::{AppView, ExportState};

/// 启动导出时逐张冻结配置，之后切换或编辑照片不会改变正在执行的任务。
struct PhotoExportJob {
    path: PathBuf,
    exif: Option<ExifInfo>,
    watermark: WatermarkPreset,
}

impl AppView {
    fn photo_export_jobs(&self, cx: &gpui_kit::App) -> Vec<PhotoExportJob> {
        self.workspace
            .photos()
            .iter()
            .map(|photo| PhotoExportJob {
                path: photo.path().to_path_buf(),
                exif: photo.exif_override().cloned(),
                watermark: self.watermark_for_photo(photo.id(), cx),
            })
            .collect()
    }

    /// 按每张照片各自的参数把队列全部导出。
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

        let photos = self.photo_export_jobs(cx);
        let total = photos.len();
        let window_handle = window.window_handle();

        self.export = ExportState::Running {
            completed: 0,
            total,
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut succeeded = 0usize;
            for (ix, job) in photos.into_iter().enumerate() {
                let result = cx
                    .background_spawn(async move {
                        let mut photo = Photo::open_blocking(&job.path)?;
                        // 只有用户编辑过时才覆盖，未编辑的照片仍使用打开文件时读到的 EXIF。
                        if let Some(exif) = job.exif {
                            photo.exif = Some(exif);
                        }
                        let watermark = photo.generate_watermark(
                            &job.watermark.params,
                            &job.watermark.text_groups,
                        )?;
                        photo.save_image(&job.watermark.params, &watermark)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::Rotation;
    use crate::ui::app::PresetScope;
    use gpui_kit::{TestAppContext, VisualTestContext};

    fn workspace(cx: &mut TestAppContext) -> (gpui_kit::Entity<AppView>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(AppView::new);
        view.update_in(cx, |view, _, cx| {
            view.add_photos(vec!["first.jpg".into(), "second.jpg".into()], cx);
        });
        (view, cx)
    }

    #[gpui_kit::test]
    fn export_freezes_each_photos_parameters_and_text(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx);
        view.update_in(cx, |view, window, cx| {
            view.load_preset("16_9", window, cx);
            view.text_groups[0].lines[0]
                .template
                .update(cx, |state, cx| {
                    state.set_value("第一张的文字", window, cx);
                });
            view.params.rotation = Rotation::Clockwise90;
            let first = view.current_watermark(cx);
            view.select_photo_at(1, window, cx);
            view.load_preset("基础样式", window, cx);
            view.text_groups[0].lines[0]
                .template
                .update(cx, |state, cx| {
                    state.set_value("第二张的文字", window, cx);
                });
            let second = view.current_watermark(cx);
            // 旧快照内的本机环境值不得覆盖最新的输出设置。
            let output = Some(PathBuf::from("/tmp/lumen-frame-test-export"));
            view.params.output_folder = output.clone();
            view.params.default_font = "Menlo".into();
            let jobs = view.photo_export_jobs(cx);
            assert_eq!(jobs.len(), 2);
            assert_eq!(jobs[0].path, PathBuf::from("first.jpg"));
            assert_eq!(
                jobs[0].watermark.params.aspect_ratio,
                first.params.aspect_ratio
            );
            assert_eq!(jobs[0].watermark.params.rotation, first.params.rotation);
            assert_eq!(jobs[0].watermark.text_groups, first.text_groups);
            assert_eq!(jobs[1].watermark.text_groups, second.text_groups);
            assert_eq!(
                jobs[1].watermark.params.aspect_ratio,
                second.params.aspect_ratio
            );
            for job in &jobs {
                assert_eq!(job.watermark.params.output_folder, output);
                assert_eq!(job.watermark.params.default_font, "Menlo");
            }
            view.load_preset("白色边框", window, cx);
            assert_eq!(jobs[1].watermark.text_groups, second.text_groups);
            view.select_photo_at(0, window, cx);
            assert_eq!(view.build_text_groups(cx), first.text_groups);
            assert_eq!(view.params.output_folder, output);
            assert_eq!(view.params.default_font, "Menlo");
        });
    }

    #[gpui_kit::test]
    fn global_preset_copies_are_independent_and_seed_new_photos(cx: &mut TestAppContext) {
        let (view, cx) = workspace(cx);
        view.update_in(cx, |view, window, cx| {
            view.load_preset("基础样式", window, cx);
            view.select_photo_at(1, window, cx);
            view.load_preset("白色边框", window, cx);
            view.preset_scope = PresetScope::AllPhotos;
            view.load_preset("16_9", window, cx);
            let global = view.current_watermark(cx);
            for job in view.photo_export_jobs(cx) {
                assert_eq!(job.watermark, global);
            }
            // 「全部照片」只控制载入预设的范围；之后的编辑也应仅影响当前照片。
            view.params.rotation = Rotation::Clockwise90;
            view.text_groups.clear();
            view.refresh_preview(cx);
            let edited = view.current_watermark(cx);
            view.add_photos(vec!["third.jpg".into()], cx);
            view.select_photo_at(2, window, cx);
            assert_eq!(view.current_watermark(cx), global);
            view.select_photo_at(0, window, cx);
            assert_eq!(view.current_watermark(cx), global);
            view.select_photo_at(1, window, cx);
            assert_eq!(view.current_watermark(cx), edited);
            view.select_photo_at(0, window, cx);
            view.remove_selected(window, cx);
            assert_eq!(view.current_watermark(cx), edited);
            assert_eq!(view.photo_watermarks.len(), 1);
            view.clear_photos(window, cx);
            assert!(view.photo_watermarks.is_empty());
            assert_eq!(view.current_watermark(cx), global);
            view.add_photos(vec!["second.jpg".into()], cx);
            assert_eq!(view.current_watermark(cx), global);
        });
    }

    #[gpui_kit::test]
    fn preset_chosen_before_import_becomes_each_photos_initial_configuration(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(AppView::new);
        view.update_in(cx, |view, window, cx| {
            view.load_preset("16_9", window, cx);
            let expected = view.current_watermark(cx);
            view.add_photos(vec!["first.jpg".into(), "second.jpg".into()], cx);
            view.params.rotation = Rotation::Clockwise90;
            view.select_photo_at(1, window, cx);
            assert_eq!(view.current_watermark(cx), expected);
            // 恢复默认只作用于当前照片，与预设应用范围无关。
            view.preset_scope = PresetScope::AllPhotos;
            view.reset_params(window, cx);
            view.select_photo_at(0, window, cx);
            assert_eq!(view.params.rotation, Rotation::Clockwise90);
            assert_eq!(view.params.aspect_ratio, Some((16.0, 9.0)));
            assert_eq!(view.global_watermark.params.aspect_ratio, Some((16.0, 9.0)));
        });
    }
}
