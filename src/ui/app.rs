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

use gpui_kit::component::{Theme, WindowExt as _, notification::Notification};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Context, Entity, FocusHandle, PathPromptOptions, RenderImage,
    ScrollHandle, SharedString, Subscription, Window,
};

use crate::features::motion_photo::{MotionPhotoOptions, export_motion_photo};
use crate::media::ExifInfo;
use crate::persistence::{
    presets::{self, WatermarkPreset},
    settings::{self as settings_store, AppearanceMode},
};
use crate::watermark::{DEFAULT_FONT, TextGroup, WatermarkParams};
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

/// 预设的应用范围；手动调整参数始终只修改当前照片。
#[derive(Clone, Copy, PartialEq, Eq)]
enum PresetScope {
    CurrentPhoto,
    AllPhotos,
}

pub struct AppView {
    page: AppPage,
    /// 无 UI 依赖的照片队列、稳定身份和选择策略。
    workspace: PhotoWorkspace,
    /// 队列键盘操作与滚动位置属于界面状态，不进入照片工作区模型。
    queue_focus: FocusHandle,
    queue_scroll: ScrollHandle,
    /// 按照片身份缓存的 UI 位图状态；它不属于工作区的领域数据。
    thumbnails: HashMap<PhotoId, Thumbnail>,
    /// 当前照片的编辑真值，供预览与导出使用。
    params: WatermarkParams,
    /// 未选中照片的配置快照。选中时移出，离开时写回，避免保存第二份编辑真值。
    photo_watermarks: HashMap<PhotoId, WatermarkPreset>,
    /// 全局预设也是后续导入照片的初始配置，不随单张照片的编辑变化。
    global_watermark: WatermarkPreset,
    preset_scope: PresetScope,
    /// 每个文字组各自持有组级控件与文字行；稳定 id 不随增删其它组改变。
    text_groups: Vec<TextGroupEditor>,
    /// 每个文字组最多打开一个独立编辑窗口；已关闭的句柄会在下次打开时清理。
    text_editor_windows: HashMap<u64, AnyWindowHandle>,
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
    logos: page::logos::LogoPageState,
    export: ExportState,
    photo_import: behavior::import::PhotoImportState,

    preset_names: Vec<SharedString>,
    builtin_presets_collapsed: bool,
    /// 只包含用户预设；新增项追加，删除项在目录刷新时清理。
    preset_order: Vec<String>,
    /// 拖动落点的预设名与前/后位置，不写入配置。
    preset_drop_target: Option<(SharedString, bool)>,
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
    /// 由持久化层提供的配置路径，载入与保存始终使用同一个位置。
    settings_path: Option<PathBuf>,
    update_state: UpdateState,

    /// 控件订阅。持有它们本身就是目的：条目在，订阅才活着。
    _subscriptions: Vec<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_settings_path(settings_store::settings_path(), window, cx)
    }

    fn new_with_settings_path(
        settings_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        component::queue::init(cx);
        component::location_picker::init(cx);
        let settings = settings_path
            .as_deref()
            .map(settings_store::load_at)
            .unwrap_or_default();
        let mut params = WatermarkParams::default();
        if let Some(folder) = settings.output_folder.clone() {
            params.output_folder = Some(folder);
        }
        params.default_font = if settings.default_font.trim().is_empty() {
            DEFAULT_FONT.to_owned()
        } else {
            settings.default_font.clone()
        };
        let text_group = TextGroup::default();

        let preview = cx.new(|_| WatermarkPreview::new());
        let gainmap = GainMapPageState::new(cx);
        let other_tools = OtherToolsState::new(window, cx);
        let logos = page::logos::LogoPageState::new();
        params.custom_logos = crate::persistence::logos::snapshot(&logos.assets);
        let global_watermark = preset_of(&params, std::slice::from_ref(&text_group));
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

        let (preset_names, preset_previews, builtin_preset_names) =
            preset_catalog(&settings.preset_order);
        let preset_order = user_preset_order(&preset_names, &builtin_preset_names);

        // 内置配色要先装进注册表，后面的下拉和 `find` 才有东西可选。
        crate::theme::install(cx);
        let (settings_controls, settings_subscriptions) = SettingsControls::new(
            settings.preview_background,
            settings::preview_max_edge_from_settings(settings.preview_max_edge),
            settings.light_theme.as_deref(),
            settings.dark_theme.as_deref(),
            &params.default_font,
            &font_names,
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
            queue_focus: cx.focus_handle(),
            queue_scroll: ScrollHandle::new(),
            thumbnails: HashMap::new(),
            params,
            photo_watermarks: HashMap::new(),
            global_watermark,
            preset_scope: PresetScope::CurrentPhoto,
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
            logos,
            export: ExportState::Idle,
            photo_import: Default::default(),
            preset_names,
            builtin_presets_collapsed: settings.builtin_presets_collapsed,
            preset_order,
            preset_drop_target: None,
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
            settings_path,
            update_state: UpdateState::Idle,
            _subscriptions: subscriptions,
        };
        // 主题要在第一帧之前落好，否则会先闪一下默认的浅色。
        settings::apply_interface_scale(view.interface_scale, window, cx);
        view.apply_theme_slots(cx);
        view.apply_appearance(window, cx);
        view.start_automatic_update_check(window, cx);
        view.load_logo_thumbnails(cx);
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

    /// 按需压平当前预览，供完整位图读取；正常窗口直接绘制缓存图层。
    /// 还没算出来时是 `None`。
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
            .map(|group| {
                let mut value = group.to_group(cx);
                value.attachment = group.attachment_target.and_then(|id| {
                    self.text_groups
                        .iter()
                        .position(|target| target.id == id)
                        .map(|target| group.attachment(target, cx))
                });
                value
            })
            .collect()
    }

    fn current_watermark(&self, cx: &App) -> WatermarkPreset {
        preset_of(&self.params, &self.build_text_groups(cx))
    }

    /// 导出读取每张照片的配置；本机环境设置始终使用当前值。
    fn watermark_for_photo(&self, id: PhotoId, cx: &App) -> WatermarkPreset {
        let mut watermark = if self.selected_photo_id() == Some(id) {
            self.current_watermark(cx)
        } else {
            self.photo_watermarks
                .get(&id)
                .unwrap_or(&self.global_watermark)
                .clone()
        };
        watermark.params.output_folder = self.params.output_folder.clone();
        watermark.params.default_font = self.params.default_font.clone();
        watermark.params.custom_logos = self.params.custom_logos.clone();
        watermark
    }

    fn save_current_watermark(&mut self, cx: &App) {
        let watermark = self.current_watermark(cx);
        if let Some(id) = self.selected_photo_id() {
            self.photo_watermarks.insert(id, watermark);
        } else {
            self.global_watermark = watermark;
        }
    }

    fn restore_selected_watermark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let watermark = self
            .selected_photo_id()
            .and_then(|id| self.photo_watermarks.remove(&id))
            .unwrap_or_else(|| self.global_watermark.clone());
        self.restore_watermark(watermark, window, cx);
    }

    // MARK: 预览

    /// 参数、选中项或文字水印变化后调用：把当前状态打包成一份预览请求。
    ///
    /// 请求本身只是「登记最新意图」，真正算不算、什么时候算由 [`WatermarkPreview`] 决定，
    /// 所以拖动滑块时可以放心地每帧调用。
    pub(super) fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        // 空队列时编辑的是新照片默认配置；导入照片后便各自独立。
        if self.selected_photo_id().is_none() {
            self.global_watermark = self.current_watermark(cx);
        }
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
        self.preview.update(cx, |preview, cx| {
            preview.request(
                job,
                self.text_groups.iter().map(|group| group.id).collect(),
                cx,
            )
        });
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

    /// 把外部路径加入队列；文件夹在后台递归扫描后加入其中的照片。
    ///
    /// 非图片文件和不认识的扩展名会被安静跳过：队列只放能处理的对象，否则用户要等到
    /// 预览报错才知道选错了文件。
    pub fn add_photos(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.iter().any(|path| path.is_dir()) {
            self.import_photo_folders(paths, cx);
            return;
        }
        if self.workspace.is_empty() {
            self.save_current_watermark(cx);
        }
        let mut added = Vec::new();
        for path in paths {
            if !is_supported_image(&path) {
                continue;
            }
            if let Some(id) = self.workspace.add(path.clone()) {
                self.photo_watermarks
                    .insert(id, self.global_watermark.clone());
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
            // 空队列时控件本来就在编辑全局默认配置，不必重建。
            self.photo_watermarks.remove(&first);
            self.refresh_preview(cx);
        }
        cx.notify();
    }

    pub(super) fn select_photo(
        &mut self,
        id: PhotoId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_photo_id() == Some(id) || self.workspace.photo(id).is_none() {
            return;
        }
        self.save_current_watermark(cx);
        self.workspace.select(id);
        self.restore_selected_watermark(window, cx);
        self.preset_feedback = None;
        cx.notify();
    }

    /// 按队列顺序选中第 `index` 张照片。
    ///
    /// 选中项本身由领域 id 标识；这里是按位置操作队列的一条受控通道。
    pub fn select_photo_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.workspace.photos().get(index).map(QueuedPhoto::id) {
            self.select_photo(id, window, cx);
        }
    }

    pub fn remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.remove_selected() else {
            return;
        };
        self.thumbnails.remove(&id);
        self.photo_watermarks.remove(&id);
        self.restore_selected_watermark(window, cx);
        self.preset_feedback = None;
        cx.notify();
    }

    pub fn clear_photos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.photo_import.revision = self.photo_import.revision.wrapping_add(1);
        self.photo_import.feedback = None;
        if self.workspace.clear() == 0 {
            cx.notify();
            return;
        }
        self.thumbnails.clear();
        self.photo_watermarks.clear();
        // 导出任务拥有冻结的照片快照，清空队列不结束任务，也不能解除重复导出的锁。
        if !matches!(self.export, ExportState::Running { .. }) {
            self.export = ExportState::Idle;
        }
        self.restore_selected_watermark(window, cx);
        self.preset_feedback = None;
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

    pub(in crate::ui::app) fn set_builtin_presets_collapsed(
        &mut self,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        if self.builtin_presets_collapsed == collapsed {
            return;
        }
        self.builtin_presets_collapsed = collapsed;
        self.preset_feedback = None;
        self.persist_preset_preferences();
        cx.notify();
    }

    /// 按名字移动预设，避免拖动过程中目录变化让下标指向另一项。
    pub(in crate::ui::app) fn move_preset(
        &mut self,
        source: &str,
        target: &str,
        before: bool,
        cx: &mut Context<Self>,
    ) {
        self.preset_drop_target = None;
        if self.builtin_preset_names.contains(source)
            || self.builtin_preset_names.contains(target)
            || !move_preset_name(&mut self.preset_names, source, target, before)
        {
            cx.notify();
            return;
        }
        self.preset_order = user_preset_order(&self.preset_names, &self.builtin_preset_names);
        self.preset_feedback = None;
        self.persist_preset_preferences();
        cx.notify();
    }

    pub(in crate::ui::app) fn move_preset_by(
        &mut self,
        name: &str,
        earlier: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .preset_order
            .iter()
            .position(|candidate| candidate == name)
        else {
            return;
        };
        let target_index = if earlier {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        if let Some(target) = target_index
            .and_then(|index| self.preset_order.get(index))
            .cloned()
        {
            self.move_preset(name, &target, earlier, cx);
        }
    }

    fn persist_preset_preferences(&mut self) {
        self.persist_settings_inner();
        if let Some(error) = self.settings_feedback.clone() {
            self.preset_feedback = Some(error);
            self.preset_feedback_is_error = true;
        }
    }

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
                self.refresh_preset_names();
                self.persist_preset_preferences();
            }
            Err(error) => {
                self.preset_feedback = Some(format!("{error:#}").into());
                self.preset_feedback_is_error = true;
            }
        }
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
                let all = self.preset_scope == PresetScope::AllPhotos;
                self.apply_preset(preset, window, cx);
                self.preset_feedback = Some(
                    if all {
                        format!("已将「{name}」应用到全部照片及后续导入")
                    } else if self.selected_photo_id().is_some() {
                        format!("已将「{name}」应用到当前照片")
                    } else {
                        format!("新照片将使用「{name}」")
                    }
                    .into(),
                );
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
        mut preset: WatermarkPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if preset.text_groups.is_empty() {
            preset.text_groups.push(TextGroup::default());
        }
        if self.preset_scope == PresetScope::AllPhotos {
            self.global_watermark = preset.clone();
            self.global_watermark.params.custom_text.clear();
            for photo in self.workspace.photos() {
                if Some(photo.id()) != self.selected_photo_id() {
                    let mut watermark = preset.clone();
                    watermark.params.custom_text =
                        self.watermark_for_photo(photo.id(), cx).params.custom_text;
                    self.photo_watermarks.insert(photo.id(), watermark);
                }
            }
        }
        // 预设只改样式和模板，保留每张照片自己的内容。
        preset.params.custom_text = self.params.custom_text.clone();
        self.restore_watermark(preset, window, cx);
    }

    /// 恢复照片快照，包括用户主动删空的文字组；不能把它当成待初始化的预设。
    fn restore_watermark(
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

        // 输出文件夹和默认字体是这台机器的环境设置，不跟着预设走。
        let output_folder = self.params.output_folder.clone();
        let default_font = self.params.default_font.clone();
        let custom_logos = self.params.custom_logos.clone();
        self.params = preset.params;
        self.params.output_folder = output_folder;
        self.params.default_font = default_font;
        self.params.custom_logos = custom_logos;

        let text_groups = preset.text_groups;
        let targets = text_groups
            .iter()
            .map(|group| group.attachment.as_ref().map(|a| a.target))
            .collect::<Vec<_>>();
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
        for (ix, target) in targets.into_iter().enumerate() {
            self.text_groups[ix].attachment_target = target.and_then(|target| {
                self.text_groups
                    .get(target)
                    .filter(|group| group.id != self.text_groups[ix].id)
                    .map(|group| group.id)
            });
        }

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
        controls.custom_text.update(cx, |state, cx| {
            state.set_value(params.custom_text.clone(), window, cx);
        });

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
        // 输出文件夹是这台机器的设置，恢复画面效果不该把它一起清掉（`restore_watermark` 会保留）。
        let mut preset = preset_of(&WatermarkParams::default(), &[TextGroup::default()]);
        preset.params.custom_text = self.params.custom_text.clone();
        self.restore_watermark(preset, window, cx);
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
                self.refresh_preset_names();
                self.persist_preset_preferences();
            }
            Err(error) => {
                self.preset_feedback = Some(format!("{error:#}").into());
                self.preset_feedback_is_error = true;
            }
        }
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
        let (names, previews, builtins) = preset_catalog(&self.preset_order);
        self.preset_order = user_preset_order(&names, &builtins);
        self.preset_names = names;
        self.preset_previews = previews;
        self.builtin_preset_names = builtins;
    }
}

fn preset_catalog(
    order: &[String],
) -> (
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

    let mut user_names = presets::list().unwrap_or_default();
    sort_preset_names(&mut user_names, order);
    for name in user_names {
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

fn user_preset_order(names: &[SharedString], builtins: &HashSet<SharedString>) -> Vec<String> {
    names
        .iter()
        .filter(|name| !builtins.contains(*name))
        .map(ToString::to_string)
        .collect()
}

/// 稳定排序让未保存过顺序的新项留在末尾，并保留目录原有的字母顺序。
fn sort_preset_names(names: &mut [String], order: &[String]) {
    names.sort_by_key(|name| {
        order
            .iter()
            .position(|saved| saved == name)
            .unwrap_or(usize::MAX)
    });
}

fn move_preset_name(
    names: &mut Vec<SharedString>,
    source: &str,
    target: &str,
    before: bool,
) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = names.iter().position(|name| name.as_ref() == source) else {
        return false;
    };
    let Some(target_index) = names.iter().position(|name| name.as_ref() == target) else {
        return false;
    };
    let slot = target_index + usize::from(!before);
    let destination = slot - usize::from(source_index < slot);
    if destination == source_index {
        return false;
    }
    let name = names.remove(source_index);
    names.insert(destination, name);
    true
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

#[cfg(test)]
mod preset_tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, VisualTestContext, point, px};
    use std::{cell::RefCell, rc::Rc};

    fn workspace<'a>(
        tag: &str,
        cx: &'a mut TestAppContext,
    ) -> (Entity<AppView>, &'a mut VisualTestContext, PathBuf) {
        let path = std::env::temp_dir()
            .join(format!(
                "lumen-frame-preset-ui-{}-{tag}",
                std::process::id()
            ))
            .join("settings.toml");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        cx.update(gpui_kit::init);
        let view = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let view_slot = view.clone();
            let path = path.clone();
            move |window, cx| {
                let app = cx.new(|cx| AppView::new_with_settings_path(Some(path), window, cx));
                view_slot.borrow_mut().replace(app.clone());
                Root::new(app, window, cx)
            }
        });
        let app = view.borrow().clone().unwrap();
        app.update_in(cx, |app, _, cx| {
            // 只替换目录展示数据，不写入用户的预设目录。
            app.preset_names
                .retain(|name| app.builtin_preset_names.contains(name));
            for name in ["test-a", "test-b", "test-c"] {
                app.preset_names.push(name.into());
                app.preset_previews
                    .insert(name.into(), preset_of(&WatermarkParams::default(), &[]));
            }
            app.preset_order = user_preset_order(&app.preset_names, &app.builtin_preset_names);
            cx.notify();
        });
        cx.run_until_parked();
        (app, cx, path)
    }

    #[test]
    fn saved_order_survives_removed_and_new_presets() {
        let mut names = vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "new".to_owned(),
        ];
        sort_preset_names(
            &mut names,
            &["gone".into(), "c".into(), "a".into(), "c".into()],
        );
        assert_eq!(names, ["c", "a", "b", "new"]);
    }

    #[gpui_kit::test]
    fn builtin_group_can_collapse_and_remember_its_state(cx: &mut TestAppContext) {
        let (view, cx, path) = workspace("collapse", cx);
        assert!(cx.debug_bounds("preset-card-16_9").is_some());
        cx.update(|window, cx| {
            window.click("builtin-presets-toggle", cx);
            window.render_frame(cx);
        });
        assert!(view.read_with(cx, |app, _| app.builtin_presets_collapsed));
        assert!(cx.debug_bounds("preset-card-16_9").is_none());
        assert!(cx.debug_bounds("preset-card-test-a").is_some());
        assert!(settings_store::load_at(&path).builtin_presets_collapsed);

        // 折叠入口保持键盘可操作。
        cx.update(|window, cx| window.press("enter", cx));
        assert!(!view.read_with(cx, |app, _| app.builtin_presets_collapsed));
        assert!(cx.debug_bounds("preset-card-16_9").is_some());
        assert!(!settings_store::load_at(&path).builtin_presets_collapsed);
        cx.update(|window, cx| window.press("space", cx));
        cx.update(|window, cx| {
            let restored =
                cx.new(|cx| AppView::new_with_settings_path(Some(path.clone()), window, cx));
            assert!(restored.read(cx).builtin_presets_collapsed);
        });
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[gpui_kit::test]
    fn dragging_personal_presets_reorders_and_saves_without_loading(cx: &mut TestAppContext) {
        let (view, cx, path) = workspace("reorder", cx);
        view.update_in(cx, |app, _, cx| app.set_builtin_presets_collapsed(true, cx));
        cx.run_until_parked();
        let source = cx.debug_bounds("preset-card-test-a").unwrap();
        let target = cx.debug_bounds("preset-card-test-c").unwrap();
        cx.update(|window, cx| {
            window.drag(
                source.center(),
                point(target.center().x, target.bottom() - px(8.0)),
                cx,
            );
            window.render_frame(cx);
        });
        let order = view.read_with(cx, |app, _| {
            assert!(app.preset_feedback.is_none(), "拖动不应触发载入预设");
            app.preset_order.clone()
        });
        assert_eq!(order, ["test-b", "test-c", "test-a"]);
        assert_eq!(settings_store::load_at(&path).preset_order, order);

        // 向前拖动插入到目标上方。
        let source = cx.debug_bounds("preset-card-test-a").unwrap();
        let target = cx.debug_bounds("preset-card-test-b").unwrap();
        cx.update(|window, cx| {
            window.drag(
                source.center(),
                point(target.center().x, target.top() + px(8.0)),
                cx,
            );
            window.render_frame(cx);
        });
        assert_eq!(
            settings_store::load_at(&path).preset_order,
            ["test-a", "test-b", "test-c"]
        );

        // 拖动后卡片保持焦点，可直接用键盘再移动一项。
        cx.update(|window, cx| window.press("alt-down", cx));
        assert_eq!(
            settings_store::load_at(&path).preset_order,
            ["test-b", "test-a", "test-c"]
        );
        let names_before = view.read_with(cx, |app, _| app.preset_names.clone());
        view.update_in(cx, |app, _, cx| app.move_preset("test-a", "16_9", true, cx));
        assert_eq!(
            view.read_with(cx, |app, _| app.preset_names.clone()),
            names_before
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
