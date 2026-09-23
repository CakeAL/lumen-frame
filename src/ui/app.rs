//! 照片水印工作台的界面层。
//!
//! 界面分成自绘标题栏、顶部页面标签，以及预设、工作区、参数三块稳定区域。
//! 设置页面复用顶部标签，整体替换下方内容。
//!
//! 状态归属：
//!
//! - [`AppView`] 拥有工程状态——照片队列、选中项、水印参数、文字水印行；
//! - 各种控件实体只保存控件自身的状态（滑块位置、下拉框开合），不另存一份参数值；
//! - [`WatermarkPreview`] 拥有预览节奏与结果，是唯一会启动后台渲染的地方。

#[path = "behavior/mod.rs"]
mod behavior;
#[path = "component/mod.rs"]
mod component;
#[path = "page/mod.rs"]
mod page;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{Root, Theme, WindowExt as _, notification::Notification};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, PathPromptOptions, RenderImage, SharedString, Subscription, Window,
    WindowHandle,
};

use crate::features::motion_photo::{MotionPhotoOptions, export_motion_photo};
use crate::media::ExifInfo;
use crate::persistence::{
    presets::{self, WatermarkPreset},
    settings::{self as settings_store, AppearanceMode},
};
use crate::watermark::{TextGroup, WatermarkParams};
use crate::workspace::{PhotoId, PhotoWorkspace, QueuedPhoto};

use super::image::{PreviewJob, export_colour_gainmap, export_gainmap, render_thumbnail};
use behavior::update::UpdateState;
use component::field::index_of;
use component::inspector::{ASPECT_RATIOS, AspectRatioChoice, ParameterControls, preset_of};
use component::preview::WatermarkPreview;
use component::queue::is_supported_image;
use component::text_section::TextGroupEditor;
use page::{
    gainmap::GainMapPageState,
    other_tools::OtherToolsState,
    settings::{self, SettingsControls},
};

/// 队列卡片的缩略图状态。
///
/// 缩略图失败只影响卡片外观，所以不携带错误详情；真正的原因由选中后的预览面板说明。
pub enum Thumbnail {
    Pending,
    Ready(Arc<RenderImage>),
    Failed,
}

/// 页面。设置整体替换中间工作区，而不是叠一层浮层。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppPage {
    Watermark,
    GainMap,
    OtherTools,
    Settings,
}

/// 导出进度。导出是这一页唯一的提交动作，所以它的状态直接反映在面板底部。
pub enum ExportState {
    Idle,
    Running { completed: usize, total: usize },
    Finished { succeeded: usize, failed: usize },
}

pub struct AppView {
    page: AppPage,
    /// 无 UI 依赖的照片队列、稳定身份和选择策略。
    workspace: PhotoWorkspace,
    /// 按照片身份缓存的 UI 位图状态；它不属于工作区的领域数据。
    thumbnails: HashMap<PhotoId, Thumbnail>,
    /// 预览与导出共用的唯一一份参数。
    params: WatermarkParams,
    /// 每个文字组各自持有组级控件与文字行；稳定 id 不随增删其它组改变。
    text_groups: Vec<TextGroupEditor>,
    /// 每个文字组最多打开一个独立编辑窗口；已关闭的句柄会在下次打开时清理。
    text_editor_windows: HashMap<u64, WindowHandle<Root>>,
    /// 窗口创建会延后到当前状态更新结束；这里防止同一组在延后期间被重复打开。
    opening_text_editor_ids: HashSet<u64>,
    /// 左侧预设面板可以收起，为照片预览腾出更多空间。
    preset_panel_collapsed: bool,
    next_text_group_id: u64,
    next_text_line_id: u64,
    /// 系统里可用的字体，每行的字体下拉都从这里取。
    font_names: Vec<SharedString>,
    /// 边框宽度那组滑块是否展开。
    border_width_open: bool,
    /// 宽高比下拉当前的选择。
    aspect_choice: AspectRatioChoice,

    controls: ParameterControls,
    preview: Entity<WatermarkPreview>,
    /// 与水印工作区完全隔离的 HDR 解析页状态。
    gainmap: GainMapPageState,
    /// 黑白底图 + 彩色恢复 gain map 的独立生成页状态。
    other_tools: OtherToolsState,
    export: ExportState,

    preset_names: Vec<SharedString>,
    /// 预设卡片使用的轻量视觉快照，避免在每一帧渲染时读取磁盘。
    preset_previews: HashMap<SharedString, WatermarkPreset>,
    /// 编译进应用的预设名称；用于禁止覆盖与删除，并在卡片上标明来源。
    builtin_preset_names: HashSet<SharedString>,
    preset_feedback: Option<SharedString>,
    preset_feedback_is_error: bool,

    /// 界面明暗的选择。真正的主题落在 GPUI 的全局 `Theme` 上，这里记住的是「用户选的是
    /// 跟随系统还是指定明暗」，以及用来在设置页上显示当前选项。
    appearance: AppearanceMode,
    /// 浅色/深色两个槽位各自选了哪套配色。`None` 表示用默认。
    light_theme: Option<SharedString>,
    dark_theme: Option<SharedString>,
    /// 界面缩放的基础字号。
    interface_scale: f32,
    /// 预览底图的长边上限；它只影响预览速度和清晰度。
    preview_max_edge: i32,
    /// 照片展示区域的背景色；属于本机界面偏好，不属于导出参数。
    preview_background: [u8; 3],
    settings: SettingsControls,
    settings_feedback: Option<SharedString>,
    update_state: UpdateState,

    /// 控件订阅。持有它们本身就是目的：条目在，订阅才活着。
    _subscriptions: Vec<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = settings_store::load();
        let mut params = WatermarkParams::default();
        if let Some(folder) = settings.output_folder.clone() {
            params.output_folder = Some(folder);
        }
        let text_group = TextGroup::default();

        let preview = cx.new(|_| WatermarkPreview::new());
        let gainmap = GainMapPageState::new(cx);
        let other_tools = OtherToolsState::new(window, cx);
        let aspect_choice = aspect_choice_for(&params);
        let (controls, subscriptions) = ParameterControls::new(&params, &aspect_choice, window, cx);

        let font_names = window
            .text_system()
            .all_font_names()
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>();

        let mut next_text_line_id = 0;
        let text_groups = vec![TextGroupEditor::new(
            0,
            &text_group,
            &mut next_text_line_id,
            &font_names,
            window,
            cx,
        )];

        let (preset_names, preset_previews, builtin_preset_names) = preset_catalog();

        // 内置配色要先装进注册表，后面的下拉和 `find` 才有东西可选。
        crate::theme::install(cx);
        let (settings_controls, settings_subscriptions) = SettingsControls::new(
            settings.preview_background,
            settings::preview_max_edge_from_settings(settings.preview_max_edge),
            settings.light_theme.as_deref(),
            settings.dark_theme.as_deref(),
            window,
            cx,
        );

        // 系统在明暗之间切换时通知一次；只有「跟随系统」才需要响应。
        let mut subscriptions = subscriptions;
        subscriptions.extend(settings_subscriptions);
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if this.appearance == AppearanceMode::System {
                Theme::sync_system_appearance(Some(window), cx);
                cx.notify();
            }
        }));

        let mut view = Self {
            page: AppPage::Watermark,
            workspace: PhotoWorkspace::default(),
            thumbnails: HashMap::new(),
            params,
            text_groups,
            text_editor_windows: HashMap::new(),
            opening_text_editor_ids: HashSet::new(),
            preset_panel_collapsed: false,
            next_text_group_id: 1,
            next_text_line_id,
            font_names,
            border_width_open: false,
            aspect_choice,
            controls,
            preview,
            gainmap,
            other_tools,
            export: ExportState::Idle,
            preset_names,
            preset_previews,
            builtin_preset_names,
            preset_feedback: None,
            preset_feedback_is_error: false,
            appearance: settings.appearance,
            light_theme: settings.light_theme.map(SharedString::from),
            dark_theme: settings.dark_theme.map(SharedString::from),
            interface_scale: settings
                .interface_scale
                .unwrap_or(settings::DEFAULT_INTERFACE_SCALE),
            preview_max_edge: settings::preview_max_edge_from_settings(settings.preview_max_edge),
            preview_background: settings.preview_background,
            settings: settings_controls,
            settings_feedback: None,
            update_state: UpdateState::Idle,
            _subscriptions: subscriptions,
        };
        // 主题要在第一帧之前落好，否则会先闪一下默认的浅色。
        settings::apply_interface_scale(view.interface_scale, window, cx);
        view.apply_theme_slots(cx);
        view.apply_appearance(window, cx);
        view.start_automatic_update_check(window, cx);
        view
    }

    // MARK: 读取

    /// 队列里的照片数量。
    pub fn photo_count(&self) -> usize {
        self.workspace.len()
    }

    /// 当前配置里的 EXIF 文字组数量。
    pub fn text_group_count(&self) -> usize {
        self.text_groups.len()
    }

    /// 已创建的文字组编辑窗口句柄数量。
    pub fn text_editor_window_count(&self) -> usize {
        self.text_editor_windows.len()
    }

    /// 当前选中的照片路径。
    pub fn selected_path(&self) -> Option<&std::path::Path> {
        self.selected_photo().map(QueuedPhoto::path)
    }

    /// 当前用于显示的预览位图；还没算出来时是 `None`。
    pub fn preview_image(&self, cx: &App) -> Option<Arc<RenderImage>> {
        self.preview.read(cx).state().image().cloned()
    }

    /// 当前的导出进度。
    pub fn export_state(&self) -> &ExportState {
        &self.export
    }

    /// 已保存的预设名。
    pub fn preset_names(&self) -> &[SharedString] {
        &self.preset_names
    }

    /// 当前参数，供预览与预设读取。
    pub fn params(&self) -> &WatermarkParams {
        &self.params
    }

    pub(super) fn selected_photo(&self) -> Option<&QueuedPhoto> {
        self.workspace.selected_photo()
    }

    pub(super) fn photos(&self) -> &[QueuedPhoto] {
        self.workspace.photos()
    }

    pub(super) fn selected_photo_id(&self) -> Option<PhotoId> {
        self.workspace.selected_id()
    }

    pub(super) fn thumbnail(&self, id: PhotoId) -> Option<&Thumbnail> {
        self.thumbnails.get(&id)
    }

    /// 由控件状态拼出渲染用的文字组。
    ///
    /// 控件实体是文字组的真值来源，领域结构只在预览、导出和保存预设时投影生成。
    pub(super) fn build_text_groups(&self, cx: &App) -> Vec<TextGroup> {
        self.text_groups
            .iter()
            .map(|group| group.to_group(cx))
            .collect()
    }

    // MARK: 预览

    /// 参数、选中项或文字水印变化后调用：把当前状态打包成一份预览请求。
    ///
    /// 请求本身只是「登记最新意图」，真正算不算、什么时候算由 [`WatermarkPreview`] 决定，
    /// 所以拖动滑块时可以放心地每帧调用。
    pub(super) fn refresh_preview(&self, cx: &mut Context<Self>) {
        let job = match self.selected_photo() {
            Some(photo) => PreviewJob {
                path: photo.path().to_path_buf(),
                exif: photo.exif().cloned(),
                params: self.params.clone(),
                text_groups: self.build_text_groups(cx),
                max_edge: self.preview_max_edge,
            },
            None => {
                self.preview.update(cx, |preview, cx| preview.clear(cx));
                return;
            }
        };
        self.preview
            .update(cx, |preview, cx| preview.request(job, cx));
    }

    pub(super) fn set_gainmap_view(&mut self, show_map: bool, cx: &mut Context<Self>) {
        self.gainmap.set_show_gainmap(show_map, cx);
        cx.notify();
    }

    pub(super) fn export_selected_gainmap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.gainmap.path().map(PathBuf::from) else {
            return;
        };
        let window_handle = window.window_handle();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择 Gain Map 保存文件夹".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let output = folder.join(format!(
                "{}_gainmap.png",
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image")
            ));
            let result = cx
                .background_spawn(async move {
                    export_gainmap(&path, &output).map(|found| (found, output))
                })
                .await;
            let _ = this.update(cx, |_this, cx| {
                let message = match result {
                    Ok((true, output)) => {
                        let message = format!("Gain Map 已导出到 {}", output.display());
                        let _ = window_handle.update(cx, |_, window, cx| {
                            window.push_notification(Notification::success(message), cx);
                        });
                        None
                    }
                    Ok((false, _)) => Some("这张图片没有 Gain Map，无法导出。".to_string()),
                    Err(error) => Some(format!("导出 Gain Map 失败：{error:#}")),
                };
                if let Some(message) = message {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn export_colour_gainmap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.other_tools.path().map(PathBuf::from) else {
            return;
        };
        if self.other_tools.is_exporting() {
            return;
        }
        let rotation = self.other_tools.colour_rotation();
        let window_handle = window.window_handle();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择保存文件夹".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let output = folder.join(format!(
                "{}_color_gainmap.jpg",
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image")
            ));
            this.update(cx, |this, cx| this.other_tools.set_exporting(true, cx))
                .ok();
            let result = cx
                .background_spawn(async move {
                    export_colour_gainmap(&path, &output, rotation).map(|_| output)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.other_tools.set_exporting(false, cx);
                let notification = match result {
                    Ok(output) => Notification::success(format!(
                        "黑白+彩色 Gain Map 已导出到 {}",
                        output.display()
                    )),
                    Err(error) => {
                        Notification::error(format!("生成黑白+彩色 Gain Map 失败：{error:#}"))
                    }
                };
                let _ = window_handle.update(cx, |_, window, cx| {
                    window.push_notification(notification, cx)
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn export_motion_photo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.other_tools.video_path().map(PathBuf::from) else {
            return;
        };
        if self.other_tools.is_motion_exporting() {
            return;
        }
        let (start, end, cover) = (
            self.other_tools.motion_start(),
            self.other_tools.motion_end(),
            self.other_tools.motion_cover(),
        );
        let max_output_size = self.other_tools.motion_max_size_bytes();
        let rotation = self.other_tools.motion_rotation();
        let Some(ffmpeg_path) = self.other_tools.ffmpeg_path().map(PathBuf::from) else {
            return;
        };
        let window_handle = window.window_handle();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择 Motion Photo 保存文件夹".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let output = folder.join(format!(
                "{}_motion_photo.jpg",
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("video")
            ));
            this.update(cx, |this, cx| {
                this.other_tools.set_motion_exporting(true, cx)
            })
            .ok();
            let result = cx
                .background_spawn(async move {
                    let options = MotionPhotoOptions {
                        ffmpeg_path: &ffmpeg_path,
                        video_path: &path,
                        output_path: &output,
                        start,
                        end,
                        cover_time: cover,
                        jpeg_quality: 92,
                        rotation,
                        max_output_size,
                    };
                    export_motion_photo(&options).map(|_| output)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.other_tools.set_motion_exporting(false, cx);
                let notification = match result {
                    Ok(output) => {
                        Notification::success(format!("Motion Photo 已导出到 {}", output.display()))
                    }
                    Err(error) => Notification::error(format!("生成 Motion Photo 失败：{error:#}")),
                };
                let _ = window_handle.update(cx, |_, window, cx| {
                    window.push_notification(notification, cx)
                });
                cx.notify();
            });
        })
        .detach();
    }

    // MARK: 页面与照片

    pub fn go_to(&mut self, page: AppPage, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            cx.notify();
        }
    }

    pub(super) fn toggle_preset_panel(&mut self, cx: &mut Context<Self>) {
        self.preset_panel_collapsed = !self.preset_panel_collapsed;
        cx.notify();
    }

    /// 用系统文件选择器挑照片。
    pub(super) fn pick_photos(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("选择照片".into()),
        });

        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            this.update(cx, |this, cx| this.add_photos(paths, cx)).ok();
        })
        .detach();
    }

    /// 通过系统选择器选择一张图片来解析 gain map。
    pub(super) fn pick_gainmap_photo(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择要解析的 HDR 照片".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            this.update(cx, |this, cx| this.add_gainmap_photo(paths, cx))
                .ok();
        })
        .detach();
    }

    /// 通过系统选择器选择一张照片来生成彩色恢复 Gain Map。
    pub(super) fn pick_colour_gainmap_photo(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择要生成彩色恢复 Gain Map 的照片".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            this.update(cx, |this, cx| this.add_colour_gainmap_photo(paths, cx))
                .ok();
        })
        .detach();
    }

    /// Motion Photo 依赖 FFmpeg，可读取其支持的视频格式。
    pub(super) fn pick_motion_photo_video(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let window_handle = window.window_handle();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 MP4（H.264、HEVC 或 AV1）视频".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let _ = window_handle.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.add_motion_photo_video(paths, window, cx)
                })
                .ok();
            });
        })
        .detach();
    }

    pub(super) fn pick_ffmpeg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let window_handle = window.window_handle();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 FFmpeg 可执行文件".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = window_handle.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    if let Err(error) = this.other_tools.set_ffmpeg_path(path, window, cx) {
                        window.push_notification(
                            Notification::error(format!("无法使用 FFmpeg：{error:#}")),
                            cx,
                        );
                    }
                })
                .ok();
            });
        })
        .detach();
    }

    /// Gain Map 页面一次只解析一张图；拖入多张时明确使用第一张支持的图片。
    pub(super) fn add_gainmap_photo(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(path) = paths.into_iter().find(|path| is_supported_image(path)) else {
            cx.notify();
            return;
        };
        self.gainmap.select(path, cx);
        cx.notify();
    }

    pub(super) fn add_colour_gainmap_photo(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(path) = paths.into_iter().find(|path| is_supported_image(path)) else {
            cx.notify();
            return;
        };
        self.other_tools.select(path, cx);
        cx.notify();
    }

    pub(super) fn add_motion_photo_video(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = paths.into_iter().find(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
        }) else {
            cx.notify();
            return;
        };
        if let Err(error) = self.other_tools.select_motion_video(path, window, cx) {
            window.push_notification(
                Notification::error(format!("无法使用 Motion Photo 视频：{error:#}")),
                cx,
            );
            cx.notify();
        }
    }

    /// 选择并保存本机导出目录；它不属于水印预设。
    pub(super) fn pick_output_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择输出文件夹".into()),
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.set_output_folder(path.to_string_lossy().into_owned());
                let _ = window_handle.update(cx, |_, window, cx| {
                    this.controls.output_folder.update(cx, |state, cx| {
                        state.set_value(path.to_string_lossy().into_owned(), window, cx)
                    });
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn set_output_folder(&mut self, value: String) {
        self.params.output_folder = (!value.trim().is_empty()).then(|| PathBuf::from(value));
        self.persist_settings_inner();
    }

    /// 把外部路径加入队列。
    ///
    /// 非图片文件和不认识的扩展名会被安静跳过：队列只放能处理的对象，否则用户要等到
    /// 预览报错才知道选错了文件。
    pub fn add_photos(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut added = Vec::new();
        for path in paths {
            if !is_supported_image(&path) {
                continue;
            }
            if let Some(id) = self.workspace.add(path.clone()) {
                self.thumbnails.insert(id, Thumbnail::Pending);
                added.push((id, path));
            }
        }

        if added.is_empty() {
            return;
        }

        let first = added[0].0;
        for (id, path) in added {
            self.load_thumbnail(id, path.clone(), cx);
            self.load_exif(id, path, cx);
        }

        if self.workspace.selected_id().is_none() {
            self.workspace.select(first);
            self.refresh_preview(cx);
        }
        cx.notify();
    }

    pub(super) fn select_photo(&mut self, id: PhotoId, cx: &mut Context<Self>) {
        if !self.workspace.select(id) {
            return;
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    /// 按队列顺序选中第 `index` 张照片。
    ///
    /// 选中项本身由领域 id 标识；这里是按位置操作队列的一条受控通道。
    pub fn select_photo_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.workspace.select_at(index) {
            return;
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    pub fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.remove_selected() else {
            return;
        };
        self.thumbnails.remove(&id);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub fn clear_photos(&mut self, cx: &mut Context<Self>) {
        if self.workspace.clear() == 0 {
            return;
        }
        self.thumbnails.clear();
        self.export = ExportState::Idle;
        self.refresh_preview(cx);
        cx.notify();
    }

    /// 缩略图单独一条后台任务：它只影响卡片外观，不该拖慢加入队列的响应。
    fn load_thumbnail(&self, id: PhotoId, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_spawn(async move { render_thumbnail(&path) })
                .await;
            this.update(cx, |this, cx| {
                if this.workspace.photo(id).is_none() {
                    return;
                }
                this.thumbnails.insert(
                    id,
                    match rendered {
                        Ok(image) => Thumbnail::Ready(image),
                        Err(_) => Thumbnail::Failed,
                    },
                );
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// EXIF 决定文字水印的排版高度，所以它到位之后预览必须重算一次。
    fn load_exif(&self, id: PhotoId, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let exif = cx
                .background_spawn(async move { ExifInfo::read(&path).ok() })
                .await;

            this.update(cx, |this, cx| {
                let is_selected = this.workspace.selected_id() == Some(id);
                let changed = this
                    .workspace
                    .photo_mut(id)
                    .is_some_and(|photo| photo.set_loaded_exif(exif));
                if is_selected && changed {
                    this.refresh_preview(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // MARK: 宽高比

    /// 选中宽高比选项。自定义时沿用输入框里现有的比值。
    pub(in crate::ui::app) fn apply_aspect_choice(&mut self, choice: AspectRatioChoice, cx: &App) {
        self.params.aspect_ratio = match &choice {
            AspectRatioChoice::Free => None,
            AspectRatioChoice::Preset(width, height) => Some((*width, *height)),
            AspectRatioChoice::Custom => self.custom_ratio(cx).or(Some((1.0, 1.0))),
        };
        self.aspect_choice = choice;
    }

    /// 自定义输入框变化后重算比例。只有处于自定义模式时才生效。
    pub(super) fn apply_custom_aspect_ratio(&mut self, cx: &mut Context<Self>) {
        if self.aspect_choice != AspectRatioChoice::Custom {
            return;
        }
        if let Some(ratio) = self.custom_ratio(cx) {
            self.params.aspect_ratio = Some(ratio);
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    fn custom_ratio(&self, cx: &App) -> Option<(f64, f64)> {
        let width = parse_ratio_part(&self.controls.aspect_width.read(cx).value())?;
        let height = parse_ratio_part(&self.controls.aspect_height.read(cx).value())?;
        Some((width, height))
    }

    // MARK: 预设

    /// 把当前配置按输入框里的名字保存成预设。
    pub(super) fn save_preset(&mut self, cx: &mut Context<Self>) {
        let name = self
            .controls
            .preset_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            return;
        }

        let preset = preset_of(&self.params, &self.build_text_groups(cx));
        match presets::save(&name, &preset) {
            Ok(path) => {
                self.preset_feedback = Some(format!("已保存到 {}", path.display()).into());
                self.preset_feedback_is_error = false;
            }
            Err(error) => {
                self.preset_feedback = Some(format!("{error:#}").into());
                self.preset_feedback_is_error = true;
            }
        }
        self.refresh_preset_names();
        cx.notify();
    }

    /// 载入预设，并把所有「自己存值」的控件同步到新配置上。
    pub(super) fn load_preset(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let preset = if self.builtin_preset_names.contains(name) {
            presets::load_builtin(name)
        } else {
            presets::load(name)
        };
        match preset {
            Ok(preset) => {
                self.apply_preset(preset, window, cx);
                self.preset_feedback = Some(format!("已载入「{name}」").into());
                self.preset_feedback_is_error = false;
            }
            Err(error) => {
                self.preset_feedback = Some(format!("{error:#}").into());
                self.preset_feedback_is_error = true;
            }
        }
        cx.notify();
    }

    /// 用一份预设替换当前配置。
    fn apply_preset(
        &mut self,
        preset: WatermarkPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor_windows = std::mem::take(&mut self.text_editor_windows);
        self.opening_text_editor_ids.clear();
        if !editor_windows.is_empty() {
            cx.defer(move |cx| {
                for handle in editor_windows.into_values() {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            });
        }

        // 输出文件夹是这台机器的环境设置，不跟着预设走。
        let output_folder = self.params.output_folder.clone();
        self.params = preset.params;
        self.params.output_folder = output_folder;

        let text_groups = if preset.text_groups.is_empty() {
            vec![TextGroup::default()]
        } else {
            preset.text_groups
        };
        let mut editors = Vec::with_capacity(text_groups.len());
        for text_group in text_groups {
            let id = self.next_text_group_id;
            self.next_text_group_id += 1;
            editors.push(TextGroupEditor::new(
                id,
                &text_group,
                &mut self.next_text_line_id,
                &self.font_names,
                window,
                cx,
            ));
        }
        self.text_groups = editors;

        self.sync_controls(window, cx);
        self.refresh_preview(cx);
    }

    /// 把所有存了值的控件拉到当前参数上。
    ///
    /// 参数是唯一真值来源，控件只是它的入口；载入预设相当于从外部改写了参数，所以每个
    /// 入口都要重新对齐一次。
    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let params = self.params.clone();
        let controls = &self.controls;

        controls.border_top.sync(params.border_ratio.0, window, cx);
        controls
            .border_bottom
            .sync(params.border_ratio.1, window, cx);
        controls.border_left.sync(params.border_ratio.2, window, cx);
        controls
            .border_right
            .sync(params.border_ratio.3, window, cx);
        controls
            .border_radius
            .sync(params.border_radius, window, cx);
        controls.shadow_size.sync(params.shadow_size, window, cx);
        controls
            .shadow_density
            .sync(params.shadow_density, window, cx);
        controls.blur_sigma.sync(params.blur_sigma, window, cx);
        controls.quality.sync(f64::from(params.quality), window, cx);
        controls.background.sync(params.background, window, cx);

        self.aspect_choice = aspect_choice_for(&params);
        let aspect_choice = self.aspect_choice.clone();
        controls.aspect_ratio.update(cx, |state, cx| {
            state.set_selected_index(index_of(ASPECT_RATIOS, &aspect_choice), window, cx)
        });
        controls.position.update(cx, |state, cx| {
            state.set_selected_value(&params.position, window, cx)
        });
        // 自定义比例的两个框：不在自定义模式时也同步，切过去就能直接用。
        let (width, height) = params.aspect_ratio.unwrap_or((1.0, 1.0));
        controls.aspect_width.update(cx, |state, cx| {
            state.set_value(format_ratio(width), window, cx)
        });
        controls.aspect_height.update(cx, |state, cx| {
            state.set_value(format_ratio(height), window, cx)
        });
    }

    /// 弹确认框再把水印参数恢复默认。
    ///
    /// 和设置页的「恢复默认设置」是两件事：那边重置的是界面偏好，这边重置的是画面效果。
    /// 这一下会丢掉手工调过的所有参数，所以先问一句。
    pub(super) fn confirm_reset_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title("恢复默认参数？")
                .description("当前的水印参数与文字水印会被默认值替换。已保存的预设不受影响。")
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("恢复默认")
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                        .on_ok(move |_, window, cx| {
                            view.update(cx, |this, cx| this.reset_params(window, cx));
                            true
                        }),
                )
        });
    }

    /// 把水印参数与文字水印恢复成默认值。
    ///
    /// 直接复用预设那条路径：默认值本来就可以看成一个内置预设，这样控件同步、文字行重建、
    /// 预览重算都不用再写一遍。
    pub(super) fn reset_params(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 输出文件夹是这台机器的设置，恢复画面效果不该把它一起清掉（`apply_preset` 会保留）。
        let preset = preset_of(&WatermarkParams::default(), &[TextGroup::default()]);
        self.apply_preset(preset, window, cx);
        cx.notify();
    }

    /// 弹一个确认框再删预设：文件删掉就找不回来了。
    pub(super) fn confirm_delete_preset(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.builtin_preset_names.contains(name) {
            self.preset_feedback = Some("内置预设不可删除".into());
            self.preset_feedback_is_error = true;
            cx.notify();
            return;
        }
        let view = cx.entity();
        let target = name.to_string();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let target = target.clone();
            alert
                .title(format!("删除预设「{target}」？"))
                .description("预设文件会从磁盘上删除，无法恢复。")
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                        .on_ok(move |_, _, cx| {
                            let target = target.clone();
                            view.update(cx, |this, cx| this.delete_preset(&target, cx));
                            true
                        }),
                )
        });
    }

    fn delete_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        match presets::delete(name) {
            Ok(()) => {
                self.preset_feedback = Some(format!("已删除「{name}」").into());
                self.preset_feedback_is_error = false;
            }
            Err(error) => {
                self.preset_feedback = Some(format!("{error:#}").into());
                self.preset_feedback_is_error = true;
            }
        }
        self.refresh_preset_names();
        cx.notify();
    }

    /// 确保预设目录存在后交给系统文件管理器显示；空目录也应可直接打开，方便手工管理。
    pub(super) fn open_preset_folder(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = presets::directory() else {
            self.preset_feedback = Some("找不到系统的配置目录".into());
            self.preset_feedback_is_error = true;
            cx.notify();
            return;
        };
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                cx.reveal_path(&dir);
                self.preset_feedback = None;
            }
            Err(error) => {
                self.preset_feedback = Some(format!("无法打开预设文件夹：{error}").into());
                self.preset_feedback_is_error = true;
            }
        }
        cx.notify();
    }

    fn refresh_preset_names(&mut self) {
        let (names, previews, builtins) = preset_catalog();
        self.preset_names = names;
        self.preset_previews = previews;
        self.builtin_preset_names = builtins;
    }
}

fn preset_catalog() -> (
    Vec<SharedString>,
    HashMap<SharedString, WatermarkPreset>,
    HashSet<SharedString>,
) {
    let builtins = presets::builtin().unwrap_or_default();
    let builtin_names = builtins
        .iter()
        .map(|(name, _)| SharedString::from(name.clone()))
        .collect::<HashSet<_>>();
    let mut names = Vec::new();
    let mut previews = HashMap::new();

    for (name, preset) in builtins {
        let name = SharedString::from(name);
        names.push(name.clone());
        previews.insert(name, preset);
    }

    for name in presets::list().unwrap_or_default() {
        let name = SharedString::from(name);
        if builtin_names.contains(&name) {
            continue;
        }
        if let Ok(preset) = presets::load(&name) {
            names.push(name.clone());
            previews.insert(name, preset);
        }
    }

    (names, previews, builtin_names)
}

/// 由参数推出宽高比下拉该选哪一项。
fn aspect_choice_for(params: &WatermarkParams) -> AspectRatioChoice {
    match params.aspect_ratio {
        None => AspectRatioChoice::Free,
        Some((width, height)) => ASPECT_RATIOS
            .iter()
            .find_map(|(_, choice)| match choice {
                AspectRatioChoice::Preset(preset_width, preset_height)
                    if (*preset_width - width).abs() < 1e-9
                        && (*preset_height - height).abs() < 1e-9 =>
                {
                    Some(choice.clone())
                }
                _ => None,
            })
            .unwrap_or(AspectRatioChoice::Custom),
    }
}

fn parse_ratio_part(text: &SharedString) -> Option<f64> {
    let value: f64 = text.trim().parse().ok()?;
    (value.is_finite() && value > 0.0).then_some(value)
}

fn format_ratio(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}
