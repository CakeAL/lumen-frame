//! 当前照片和整个队列共用的导出流程。
//!
//! [`AppView`] 仍然拥有队列、参数与进度状态；本模块只把它们组织为一次串行导出，
//! 避免 libvips 的图像任务在界面层互相争抢资源。

use std::path::PathBuf;

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{App, Context, Window, prelude::*};

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

#[derive(Clone, Copy)]
enum ExportScope {
    CurrentPhoto,
    AllPhotos,
}

impl AppView {
    fn photo_export_jobs(&self, scope: ExportScope, cx: &App) -> Vec<PhotoExportJob> {
        self.workspace
            .photos()
            .iter()
            .filter(|photo| {
                matches!(scope, ExportScope::AllPhotos)
                    || Some(photo.id()) == self.workspace.selected_id()
            })
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
        if self.photo_import.pending > 0 {
            return;
        }
        self.export_photos(ExportScope::AllPhotos, window, cx);
    }

    pub(in crate::ui::app) fn export_current(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_photos(ExportScope::CurrentPhoto, window, cx);
    }

    fn export_photos(&mut self, scope: ExportScope, window: &mut Window, cx: &mut Context<Self>) {
        let photos = self.photo_export_jobs(scope, cx);
        self.start_export(photos, window, cx, |folder, cx| cx.open_with_system(folder));
    }

    /// 文件管理器是导出的最后一个外部副作用，独立注入以验证成功、失败与配置冻结的行为。
    fn start_export(
        &mut self,
        photos: Vec<PhotoExportJob>,
        window: &mut Window,
        cx: &mut Context<Self>,
        open_folder: impl FnOnce(&std::path::Path, &mut App) + 'static,
    ) {
        if photos.is_empty() || matches!(self.export, ExportState::Running { .. }) {
            return;
        }
        let Some(output_folder) = photos[0].watermark.params.output_folder.clone() else {
            return;
        };

        let total = photos.len();
        let window_handle = window.window_handle();

        self.export = ExportState::Running {
            completed: 0,
            total,
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut succeeded = 0usize;
            let mut first_error = None;
            for (ix, job) in photos.into_iter().enumerate() {
                let path = job.path.clone();
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
                } else if first_error.is_none() {
                    first_error = result
                        .err()
                        .map(|error| format!("{}：{error:#}", path.display()));
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
                if let Some(error) = first_error {
                    let message = format!(
                        "已导出 {succeeded} 张，{} 张失败。{error}",
                        total - succeeded
                    );
                    let _ = window_handle.update(cx, |_, window, cx| {
                        window.push_notification(Notification::error(message), cx);
                    });
                } else {
                    let message = format!("已成功导出 {succeeded} 张照片");
                    let _ = window_handle.update(cx, |_, window, cx| {
                        window.push_notification(Notification::success(message), cx);
                    });
                }
                if succeeded > 0 {
                    // 使用启动时冻结的目录；中途更改输出设置不能打开另一个文件夹。
                    open_folder(&output_folder, cx);
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
    use std::{cell::RefCell, rc::Rc};

    fn rendered_workspace(
        cx: &mut TestAppContext,
    ) -> (gpui_kit::Entity<AppView>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let slot = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let slot = slot.clone();
            move |window, cx| {
                let view = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
                slot.borrow_mut().replace(view.clone());
                gpui_kit::component::Root::new(view, window, cx)
            }
        });
        let view = slot.borrow().clone().unwrap();
        (view, cx)
    }

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
            let jobs = view.photo_export_jobs(ExportScope::AllPhotos, cx);
            let current = view.photo_export_jobs(ExportScope::CurrentPhoto, cx);
            assert_eq!(current.len(), 1);
            assert_eq!(current[0].path, PathBuf::from("second.jpg"));
            assert_eq!(current[0].watermark.text_groups, second.text_groups);
            assert_eq!(current[0].watermark.params.output_folder, output);
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
            for job in view.photo_export_jobs(ExportScope::AllPhotos, cx) {
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

    #[gpui_kit::test]
    fn export_buttons_share_a_row_and_respect_selection_output_and_busy_state(
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::test::TestWindowExt as _;
        let (view, cx) = rendered_workspace(cx);
        cx.run_until_parked();
        cx.update(|window, cx| {
            let current = window.find("export-current");
            let all = window.find("export");
            assert_eq!(current.bounds().origin.y, all.bounds().origin.y);
            assert!(current.bounds().right() < all.bounds().origin.x);
            window.click("export-current", cx);
            window.click("export", cx);
            assert!(matches!(view.read(cx).export, ExportState::Idle));
        });
        view.update_in(cx, |view, _, cx| {
            view.add_photos(vec!["first.jpg".into(), "second.jpg".into()], cx);
            view.params.output_folder = None;
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("export-current", cx);
            window.click("export", cx);
            assert!(matches!(view.read(cx).export, ExportState::Idle));
        });
        view.update_in(cx, |view, _, _| {
            view.params.output_folder = Some("/tmp/lumen-frame-output".into());
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("export-current", cx);
            assert!(matches!(
                view.read(cx).export,
                ExportState::Running { total: 1, .. }
            ));
            window.render_frame(cx);
            window.click("export-current", cx);
            window.click("export", cx);
            assert!(matches!(
                view.read(cx).export,
                ExportState::Running { total: 1, .. }
            ));
        });
    }

    fn export_directory(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("lumen-frame-export-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[gpui_kit::test]
    fn single_export_writes_only_current_photo_and_opens_the_frozen_folder_once(
        cx: &mut TestAppContext,
    ) {
        let dir = export_directory("single");
        let first = dir.join("first.jpg");
        let second = dir.join("second.jpg");
        image::RgbImage::from_pixel(64, 48, image::Rgb([80, 100, 120]))
            .save(&first)
            .unwrap();
        std::fs::copy(&first, &second).unwrap();
        let output = dir.join("output");
        let opened = Rc::new(RefCell::new(Vec::new()));
        let (view, cx) = rendered_workspace(cx);
        view.update_in(cx, |view, window, cx| {
            view.add_photos(vec![first.clone(), second.clone()], cx);
            view.select_photo_at(1, window, cx);
            view.params.output_folder = Some(output.clone());
            view.params.shadow_size = 0.;
            view.params.rotation = Rotation::Clockwise90;
            view.text_groups.clear();
            let jobs = view.photo_export_jobs(ExportScope::CurrentPhoto, cx);
            let opened = opened.clone();
            view.start_export(jobs, window, cx, move |folder, _| {
                opened.borrow_mut().push(folder.to_path_buf())
            });
            // 清空队列和更改输出目录都不能改变已经启动的任务或解除重复导出的锁。
            view.params.output_folder = Some(dir.join("later"));
            view.clear_photos(window, cx);
            assert!(matches!(view.export, ExportState::Running { total: 1, .. }));
            view.add_photos(vec![first.clone()], cx);
            let jobs = view.photo_export_jobs(ExportScope::AllPhotos, cx);
            view.start_export(jobs, window, cx, |_, _| panic!("不能重复启动导出"));
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| matches!(
            view.export,
            ExportState::Finished {
                succeeded: 1,
                failed: 0
            }
        )));
        assert!(output.join("second_watermark.jpg").is_file());
        assert!(!output.join("first_watermark.jpg").exists());
        assert!(!dir.join("later").exists());
        let (width, height) = image::image_dimensions(output.join("second_watermark.jpg")).unwrap();
        assert!(height > width, "应使用选中照片的旋转配置");
        assert_eq!(&*opened.borrow(), &[output]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui_kit::test]
    fn failed_export_does_not_open_an_output_folder(cx: &mut TestAppContext) {
        let dir = export_directory("failed");
        let (view, cx) = rendered_workspace(cx);
        view.update_in(cx, |view, window, cx| {
            view.add_photos(vec![dir.join("missing.jpg")], cx);
            view.params.output_folder = Some(dir.join("output"));
            let jobs = view.photo_export_jobs(ExportScope::CurrentPhoto, cx);
            view.start_export(jobs, window, cx, |_, _| {
                panic!("没有导出图片时不能打开文件夹")
            });
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| matches!(
            view.export,
            ExportState::Finished {
                succeeded: 0,
                failed: 1
            }
        )));
        assert!(!dir.join("output").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui_kit::test]
    fn partial_batch_export_reports_failure_and_still_opens_successful_output(
        cx: &mut TestAppContext,
    ) {
        let dir = export_directory("partial");
        let source = dir.join("first.jpg");
        image::RgbImage::from_pixel(64, 48, image::Rgb([80, 100, 120]))
            .save(&source)
            .unwrap();
        let output = dir.join("output");
        let opened = Rc::new(RefCell::new(Vec::new()));
        let (view, cx) = rendered_workspace(cx);
        view.update_in(cx, |view, window, cx| {
            view.add_photos(vec![source, dir.join("missing.jpg")], cx);
            view.params.output_folder = Some(output.clone());
            view.params.shadow_size = 0.;
            view.text_groups.clear();
            let jobs = view.photo_export_jobs(ExportScope::AllPhotos, cx);
            let opened = opened.clone();
            view.start_export(jobs, window, cx, move |folder, _| {
                opened.borrow_mut().push(folder.to_path_buf())
            });
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| matches!(
            view.export,
            ExportState::Finished {
                succeeded: 1,
                failed: 1
            }
        )));
        assert!(output.join("first_watermark.jpg").is_file());
        assert_eq!(&*opened.borrow(), &[output]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
