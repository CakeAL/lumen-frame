//! 外部操作与桌面共享 Session；文件/媒体工作使用冻结快照，结果回到 GPUI 主线程提交。
use super::super::{AppView, ExportState, Thumbnail, page::settings};
use crate::backend::{
    Command, ErrorCode, OperationError, Reply, Request, Session, bridge::CommandInbox,
    presets::WatermarkPreset, workspace::PhotoId,
};
use gpui_kit::{AppContext as _, Context, SharedString, Window};

/// 调度策略属于桌面宿主：纯状态修改同步执行，I/O 在后台执行。
enum Execution {
    Foreground,
    /// 仅返回结果，不提交 Session；仍可能写入预设或导出文件。
    Background,
    BackgroundWrite,
}
fn execution(command: &Command) -> Execution {
    use Command::*;
    match command {
        GetState
        | GetWatermark { .. }
        | GetExif { .. }
        | SelectPhoto { .. }
        | RemovePhoto { .. }
        | ClearPhotos
        | SetWatermark { .. }
        | PatchParams { .. }
        | SetCustomText { .. }
        | SetEnvironment { .. }
        | ResetWatermark
        | SetTextGroups { .. }
        | AddTextGroup { .. }
        | UpdateTextGroup { .. }
        | RemoveTextGroup { .. }
        | AddTextLine { .. }
        | UpdateTextLine { .. }
        | RemoveTextLine { .. }
        | FormatTextLine { .. }
        | EditTextLine { .. }
        | ApplyCurrentToAll
        | UpdateExif { .. }
        | SetGps { .. } => Execution::Foreground,
        ImportPhotos { .. }
        | LoadPreset { .. }
        | SaveSettings { .. }
        | ImportLogos { .. }
        | DeleteLogo { .. } => Execution::BackgroundWrite,
        ExtractPalette { .. }
        | RenderPreview { .. }
        | RenderThumbnail { .. }
        | ExportPhotos { .. }
        | ListPresets
        | SavePreset { .. }
        | DeletePreset { .. }
        | GetSettings
        | ListLogos
        | ExportGainmap { .. }
        | ExportColourGainmap { .. }
        | InspectVideo { .. }
        | ExportMotionPhoto { .. }
        | ClearMapCache
        | SearchPlaces { .. }
        | DetectFfmpeg
        | ValidateFfmpeg { .. }
        | RenderGainmapPreview { .. }
        | RenderColourGainmapPreview { .. }
        | RenderMotionPhotoCover { .. }
        | CheckUpdate
        | InstallUpdate => Execution::Background,
    }
}
enum Prepared {
    Complete(Reply),
    Background(Box<Session>),
}
struct BackgroundResult {
    snapshot: Session,
    reply: Reply,
    revision: u64,
}
fn failure(revision: u64, code: ErrorCode, message: &str) -> Reply {
    Reply {
        revision,
        result: Err(OperationError {
            code,
            message: message.into(),
        }),
    }
}
impl AppView {
    /// 只由应用装配层连接一次；任务持有 WeakEntity，不延长已关闭窗口的生命周期。
    pub fn attach_command_inbox(
        &mut self,
        inbox: CommandInbox,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.external_commands = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(call) = inbox.next().await {
                if call.is_cancelled() {
                    continue;
                }
                let (request, response) = call.into_parts();
                let prepared = this.update_in(cx, |this, window, cx| {
                    this.prepare_external(&request, window, cx)
                });
                let Ok(prepared) = prepared else {
                    break;
                };
                let reply = match prepared {
                    Prepared::Complete(reply) => reply,
                    Prepared::Background(mut snapshot) => {
                        let revision = snapshot.revision();
                        let command = request.command.clone();
                        let work_command = command.clone();
                        let (snapshot, reply, catalog, logos) = cx
                            .background_spawn(async move {
                                let reply = snapshot.dispatch(request);
                                // 与领域操作在同一后台任务读取，主线程只做展示投影。
                                let catalog = if reply.result.is_ok()
                                    && matches!(
                                        work_command,
                                        Command::SavePreset { .. } | Command::DeletePreset { .. }
                                    ) {
                                    Some(super::super::preset_catalog(&[]))
                                } else {
                                    None
                                };
                                let logos = if reply.result.is_ok()
                                    && matches!(
                                        work_command,
                                        Command::ImportLogos { .. } | Command::DeleteLogo { .. }
                                    ) {
                                    snapshot.storage().logos().ok().and_then(|path| {
                                        crate::backend::logos::library_in(path).ok()
                                    })
                                } else {
                                    None
                                };
                                (snapshot, reply, catalog, logos)
                            })
                            .await;
                        let finished = this.update_in(cx, |this, window, cx| {
                            let reply = this.finish_external(
                                &command,
                                BackgroundResult {
                                    snapshot: *snapshot,
                                    reply,
                                    revision,
                                },
                                response.is_closed(),
                                window,
                                cx,
                            );
                            if reply.result.is_ok() {
                                if let Some((mut names, previews, builtins)) = catalog {
                                    names.sort_by_key(|name| {
                                        if builtins.contains(name) {
                                            (0, 0)
                                        } else {
                                            (
                                                1,
                                                this.preset_order
                                                    .iter()
                                                    .position(|saved| saved == name.as_ref())
                                                    .unwrap_or(usize::MAX),
                                            )
                                        }
                                    });
                                    this.preset_names = names;
                                    this.preset_previews = previews;
                                    this.builtin_preset_names = builtins;
                                    cx.notify();
                                }
                                if let Some(assets) = logos {
                                    this.synchronize_logos(assets, cx);
                                }
                            }
                            reply
                        });
                        let Ok(reply) = finished else {
                            break;
                        };
                        reply
                    }
                };
                let _ = response.try_send(reply);
            }
        }));
    }
    pub fn disconnect_commands(&mut self) {
        self.external_commands.take();
    }

    fn prepare_external(
        &mut self,
        request: &Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Prepared {
        self.save_current_watermark(cx);
        if request
            .expected_revision
            .is_some_and(|revision| revision != self.backend.revision())
        {
            return Prepared::Complete(failure(
                self.backend.revision(),
                ErrorCode::Conflict,
                "状态已更新，请重新读取后再提交",
            ));
        }
        if matches!(
            request.command,
            Command::ImportLogos { .. } | Command::DeleteLogo { .. }
        ) && !self.begin_external_logo_import(cx)
        {
            return Prepared::Complete(failure(
                self.backend.revision(),
                ErrorCode::OperationFailed,
                "正在导入 Logo，请等待完成后重试",
            ));
        }
        if matches!(request.command, Command::ExportPhotos { .. }) {
            if matches!(self.export, ExportState::Running { .. }) || self.photo_import.pending > 0 {
                return Prepared::Complete(failure(
                    self.backend.revision(),
                    ErrorCode::OperationFailed,
                    "照片正在导入或导出，请等待完成后重试",
                ));
            }
            let total = match request.command {
                Command::ExportPhotos { all_photos: true } => self.photo_count(),
                _ => usize::from(self.selected_photo_id().is_some()),
            };
            self.export = ExportState::Running {
                completed: 0,
                total,
            };
            cx.notify();
        }
        if matches!(execution(&request.command), Execution::Foreground) {
            let before = self.backend.current().clone();
            let selected = self.selected_photo_id();
            let revision = self.backend.revision();
            let mut reply = self.backend.dispatch(request.clone());
            // 空队列也可能有尚未完成的文件夹扫描，清空必须使其失效。
            if reply.result.is_ok() && matches!(request.command, Command::ClearPhotos) {
                self.photo_import.revision = self.photo_import.revision.wrapping_add(1);
                self.photo_import.feedback = None;
                if !matches!(self.export, ExportState::Running { .. }) {
                    self.export = ExportState::Idle;
                }
                cx.notify();
            }
            if reply.result.is_ok() && self.backend.revision() != revision {
                self.synchronize_session(&before, selected, window, cx);
                if matches!(request.command, Command::SetEnvironment { .. }) {
                    self.persist_settings();
                }
                reply.revision = self.backend.revision();
            }
            Prepared::Complete(reply)
        } else {
            Prepared::Background(Box::new(self.backend.clone()))
        }
    }

    fn finish_external(
        &mut self,
        command: &Command,
        result: BackgroundResult,
        cancelled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Reply {
        let BackgroundResult {
            snapshot,
            mut reply,
            revision,
        } = result;
        if matches!(
            command,
            Command::ImportLogos { .. } | Command::DeleteLogo { .. }
        ) {
            self.end_external_logo_import(cx);
        }
        self.backend.commit_groups(self.build_text_groups(cx));
        if matches!(command, Command::ExportPhotos { .. }) {
            self.export = match &reply.result {
                Ok(output) => ExportState::Finished {
                    succeeded: output.value()["exported_inputs"]
                        .as_array()
                        .map_or(0, Vec::len),
                    failed: output.value()["errors"].as_array().map_or(0, Vec::len),
                },
                Err(_) => ExportState::Idle,
            };
            cx.notify();
        }
        if reply.result.is_ok() && matches!(execution(command), Execution::BackgroundWrite) {
            if cancelled {
                return failure(
                    self.backend.revision(),
                    ErrorCode::OperationFailed,
                    "请求已取消，未提交桌面状态",
                );
            }
            if self.backend.revision() != revision {
                return failure(
                    self.backend.revision(),
                    ErrorCode::Conflict,
                    "处理期间界面状态已更新，结果未覆盖当前工作区；请重新读取状态。已完成的文件写入不会撤回。",
                );
            }
            let before = self.backend.current().clone();
            let selected = self.selected_photo_id();
            self.backend = snapshot;
            self.synchronize_session(&before, selected, window, cx);
            if let Command::SaveSettings { settings } = command {
                self.synchronize_settings(settings, window, cx);
            }
            reply.revision = self.backend.revision();
        }
        reply
    }

    fn synchronize_session(
        &mut self,
        before: &WatermarkPreset,
        selected: Option<PhotoId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.thumbnails
            .retain(|id, _| self.backend.workspace().photo(*id).is_some());
        self.background_state
            .palettes
            .retain(|(id, _), _| self.backend.workspace().photo(*id).is_some());
        let added = self
            .photos()
            .iter()
            .filter(|photo| !self.thumbnails.contains_key(&photo.id()))
            .map(|photo| (photo.id(), photo.path().to_path_buf()))
            .collect::<Vec<_>>();
        for (id, path) in added {
            self.thumbnails.insert(id, Thumbnail::Pending);
            self.load_thumbnail(id, path, cx);
        }
        if before.text_groups != self.backend.current().text_groups
            || selected != self.selected_photo_id()
        {
            self.rebuild_text_editors(self.backend.current().text_groups.clone(), window, cx);
        }
        self.sync_controls(window, cx);
        let output = self
            .backend
            .params()
            .output_folder
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.controls
            .output_folder
            .update(cx, |input, cx| input.set_value(output, window, cx));
        let font = SharedString::from(self.backend.params().default_font.clone());
        self.settings
            .default_font
            .update(cx, |input, cx| input.set_selected_value(&font, window, cx));
        self.refresh_preview(cx);
        self.schedule_palette_for_selected(cx);
        cx.notify();
    }

    fn synchronize_settings(
        &mut self,
        saved: &crate::backend::settings::AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::{Theme, ThemeRegistry};
        self.appearance = saved.appearance;
        self.window_effects = saved.window_effects;
        crate::window_effects::set_enabled(saved.window_effects, cx);
        self.light_theme = saved.light_theme.clone().map(Into::into);
        self.dark_theme = saved.dark_theme.clone().map(Into::into);
        self.interface_scale = saved
            .interface_scale
            .unwrap_or(settings::DEFAULT_INTERFACE_SCALE);
        self.preview_max_edge = settings::preview_max_edge_from_settings(saved.preview_max_edge);
        self.preview_background = saved.preview_background;
        self.map_settings = saved.map.clone();
        self.builtin_presets_collapsed = saved.builtin_presets_collapsed;
        self.preset_order = saved.preset_order.clone();
        let (light, dark) = {
            let registry = ThemeRegistry::global(cx);
            (
                registry.default_light_theme().clone(),
                registry.default_dark_theme().clone(),
            )
        };
        Theme::update(cx, |theme| {
            theme.apply_config(&light);
            theme.apply_config(&dark);
        });
        for (input, name) in [
            (
                &self.settings.light_theme,
                self.light_theme.as_ref().unwrap_or(&light.name),
            ),
            (
                &self.settings.dark_theme,
                self.dark_theme.as_ref().unwrap_or(&dark.name),
            ),
        ] {
            input.update(cx, |input, cx| {
                input.set_selected_value(name, window, cx);
            });
        }
        self.settings
            .preview_background
            .sync(self.preview_background, window, cx);
        self.settings
            .preview_max_edge
            .sync(f64::from(self.preview_max_edge), window, cx);
        self.apply_theme_slots(cx);
        self.apply_appearance(window, cx);
        settings::apply_interface_scale(self.interface_scale, window, cx);
        self.refresh_preview(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{bridge::command_channel, watermark::TextGroup};
    use gpui_kit::{
        Entity, TestAppContext, VisualTestContext,
        component::{Root, input::InputEvent},
    };
    use lumen_frame_mcp::ipc::{DesktopEndpoint, DesktopServer};
    use std::{cell::RefCell, rc::Rc, time::Duration};

    fn call(
        endpoint: &DesktopEndpoint,
        command: Command,
        revision: Option<u64>,
        cx: &mut VisualTestContext,
    ) -> Reply {
        let endpoint = endpoint.clone();
        let (send, recv) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            send.send(runtime.block_on(endpoint.call(Request {
                expected_revision: revision,
                command,
            })))
            .unwrap();
        });
        for _ in 0..1000 {
            cx.executor().advance_clock(Duration::from_millis(20));
            cx.run_until_parked();
            if let Ok(reply) = recv.try_recv() {
                return reply.unwrap();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("桌面 IPC 调用未返回");
    }
    #[gpui_kit::test]
    fn desktop_ipc_updates_controls_preview_and_reads_user_edits(cx: &mut TestAppContext) {
        // 真实 Socket 的唤醒来自 Tokio 线程，显式允许非确定性 I/O。
        cx.executor().allow_parking();
        let root = std::env::temp_dir().join(format!("lumen-ipc-ui-{}", std::process::id()));
        #[cfg(unix)]
        let root = std::path::PathBuf::from("/tmp").join(root.file_name().unwrap());
        let _ = std::fs::remove_dir_all(&root);
        cx.update(gpui_kit::init);
        let (client, inbox) = command_channel();
        let endpoint = DesktopEndpoint::in_directory(root.join("ipc"));
        let server = DesktopServer::start(endpoint.clone(), client).unwrap();
        let slot = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let slot = slot.clone();
            let root = root.clone();
            move |window, cx| {
                let view = cx.new(|cx| {
                    let mut view = AppView::new_with_settings_path(
                        Some(root.join("settings.toml")),
                        window,
                        cx,
                    );
                    view.attach_command_inbox(inbox, window, cx);
                    view
                });
                slot.borrow_mut().replace(view.clone());
                Root::new(view, window, cx)
            }
        });
        let view: Entity<AppView> = slot.borrow().clone().unwrap();
        let photo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test_images/ultra_hdr.jpg");
        let imported = call(
            &endpoint,
            Command::ImportPhotos { paths: vec![photo] },
            None,
            cx,
        );
        assert!(imported.result.is_ok(), "{imported:?}");
        assert_eq!(view.read_with(cx, |view, _| view.photo_count()), 1);
        let reply = call(
            &endpoint,
            Command::SetCustomText {
                text: "AI 修改".into(),
            },
            None,
            cx,
        );
        assert!(reply.result.is_ok());
        assert_eq!(
            view.read_with(cx, |view, cx| view
                .controls
                .custom_text
                .read(cx)
                .value()
                .to_string()),
            "AI 修改"
        );
        let mut group = TextGroup::default();
        group.text.template = vec!["测试 {型号}".into()];
        group.text.text_params.truncate(1);
        let reply = call(
            &endpoint,
            Command::SetTextGroups {
                groups: vec![group],
            },
            Some(reply.revision),
            cx,
        );
        assert!(reply.result.is_ok());
        assert_eq!(
            view.read_with(cx, |view, cx| view.text_groups[0].lines[0]
                .template
                .read(cx)
                .value()
                .to_string()),
            "测试 {型号}"
        );
        // 用户编辑走同一订阅路径，外部读取必须提交文字草稿。
        view.update_in(cx, |view, window, cx| {
            view.text_groups[0].lines[0]
                .template
                .update(cx, |input, cx| {
                    input.set_value("用户 {型号}", window, cx);
                    cx.emit(InputEvent::Change);
                });
        });
        let state = call(&endpoint, Command::GetState, None, cx);
        let state = state.result.unwrap();
        assert_eq!(
            state.value()["current"]["watermark"]["text_groups"][0]["text"]["template"][0],
            "用户 {型号}"
        );
        let conflict = call(
            &endpoint,
            Command::SetCustomText {
                text: "过期修改".into(),
            },
            Some(0),
            cx,
        );
        assert!(matches!(
            conflict.result.unwrap_err().code,
            ErrorCode::Conflict
        ));
        assert_eq!(
            view.read_with(cx, |view, _| view.params().custom_text.clone()),
            "AI 修改"
        );
        for _ in 0..80 {
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            if view.read_with(cx, |view, cx| view.preview_image(cx).is_some()) {
                break;
            }
        }
        assert!(view.read_with(cx, |view, cx| view.preview_image(cx).is_some()));
        let saved = call(
            &endpoint,
            Command::SaveSettings {
                settings: crate::backend::settings::AppSettings {
                    window_effects: false,
                    ..Default::default()
                },
            },
            None,
            cx,
        );
        assert!(saved.result.is_ok(), "{saved:?}");
        assert!(!view.read_with(cx, |view, _| view.window_effects));
        assert!(!cx.read(crate::window_effects::enabled));
        drop(server);
        let unavailable = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(endpoint.call(Request {
                expected_revision: None,
                command: Command::GetState,
            }));
        assert!(unavailable.is_err());
        view.update_in(cx, |view, _, _| view.disconnect_commands());
        let _ = std::fs::remove_dir_all(root);
    }

    #[gpui_kit::test]
    fn desktop_ipc_background_state_cannot_overwrite_new_ui_changes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
            Root::new(view, window, cx)
        });
        // 独立 AppView 足以覆盖快照提交的并发保护，不启动无关媒体任务。
        cx.update(|window, cx| {
            let view = cx.new(|cx| AppView::new_with_settings_path(None, window, cx));
            view.update(cx, |view, cx| {
                view.save_current_watermark(cx);
                let revision = view.backend.revision();
                let mut snapshot = view.backend.clone();
                let command = Command::LoadPreset {
                    name: "test".into(),
                    all_photos: false,
                };
                let reply = snapshot.dispatch(Request {
                    expected_revision: None,
                    command: Command::SetCustomText {
                        text: "旧快照".into(),
                    },
                });
                view.backend.params_mut().custom_text = "用户新输入".into();
                let result = view.finish_external(
                    &command,
                    BackgroundResult {
                        snapshot,
                        reply,
                        revision,
                    },
                    false,
                    window,
                    cx,
                );
                assert!(matches!(
                    result.result.unwrap_err().code,
                    ErrorCode::Conflict
                ));
                assert_eq!(view.params().custom_text, "用户新输入");
                view.photo_import.pending = 1;
                let revision = view.photo_import.revision;
                let result = view.prepare_external(
                    &Request {
                        expected_revision: None,
                        command: Command::ClearPhotos,
                    },
                    window,
                    cx,
                );
                assert!(matches!(
                    result,
                    Prepared::Complete(Reply { result: Ok(_), .. })
                ));
                assert_eq!(
                    view.photo_import.revision,
                    revision.wrapping_add(1),
                    "空队列也应取消迟到的扫描结果"
                );
            });
        });
    }
}
