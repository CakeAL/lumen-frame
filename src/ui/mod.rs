//! 照片水印工作台的界面层。
//!
//! 界面分成三块稳定区域：左侧导航、中间工作区（上：预览，下：照片队列）、右侧参数面板。
//! 设置是独立页面，切换时整体替换中间工作区。
//!
//! 状态归属：
//!
//! - [`AppView`] 拥有工程状态——照片队列、选中项、水印参数、文字水印行；
//! - 各种控件实体只保存控件自身的状态（滑块位置、下拉框开合），不另存一份参数值；
//! - [`WatermarkPreview`] 拥有预览节奏与结果，是唯一会启动后台渲染的地方。

mod field;
mod inspector;
mod preview;
mod preview_image;
mod queue;
mod settings;
mod sidebar;
mod text_section;

/// 预览位图的生成入口。
///
/// 单独公开这一层，是因为它是水印管线与 GPUI 渲染之间的接缝：它有明确的行为契约
/// （给定参数就得到确定的位图），可以脱离窗口独立测试。
pub use preview_image::{PreviewJob, render_preview, render_thumbnail};

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Root, Theme, ThemeMode, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, ExternalPaths, PathPromptOptions, RenderImage, SharedString,
    Subscription, Window, WindowAppearance,
};

use crate::Position;
use crate::config::{self, AppearanceMode, WatermarkPreset};
use crate::params::WatermarkParams;
use crate::photo::{ExifInfo, Photo};
use crate::process::text::{Text, TextParams};

use field::index_of;
use inspector::{ASPECT_RATIOS, AspectRatioChoice, ParameterControls, preset_of};
use preview::WatermarkPreview;
use queue::is_supported_image;
use text_section::TextLine;

/// 队列中一张照片的稳定身份。
///
/// 用 id 而不是下标标记选中项：移除或重排照片时，下标会让选中状态漂移到隔壁那张。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct PhotoId(u64);

/// 队列卡片的缩略图状态。
///
/// 缩略图失败只影响卡片外观，所以不携带错误详情；真正的原因由选中后的预览面板说明。
pub enum Thumbnail {
    Pending,
    Ready(Arc<RenderImage>),
    Failed,
}

pub struct QueuedPhoto {
    pub id: PhotoId,
    pub path: PathBuf,
    pub exif: Option<ExifInfo>,
    pub thumbnail: Thumbnail,
}

/// 页面。设置整体替换中间工作区，而不是叠一层浮层。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppPage {
    Watermark,
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
    photos: Vec<QueuedPhoto>,
    selected: Option<PhotoId>,
    next_photo_id: u64,
    /// 预览与导出共用的唯一一份参数。
    params: WatermarkParams,
    /// 文字水印的整体设置；每行自己的参数在 `text_lines` 里。
    text_position: Position,
    time_format: String,
    text_lines: Vec<TextLine>,
    next_text_line_id: u64,
    /// 系统里可用的字体，每行的字体下拉都从这里取。
    font_names: Vec<SharedString>,
    /// 展开的文字行，按行 id 记录：删掉一行之后，展开状态不会串到别的行上。
    expanded_text_lines: Vec<u64>,
    /// 边框宽度那组滑块是否展开。
    border_width_open: bool,
    /// 宽高比下拉当前的选择。
    aspect_choice: AspectRatioChoice,

    controls: ParameterControls,
    preview: Entity<WatermarkPreview>,
    export: ExportState,

    preset_names: Vec<SharedString>,
    preset_feedback: Option<SharedString>,
    preset_feedback_is_error: bool,

    /// 界面明暗的选择。真正的主题落在 GPUI 的全局 `Theme` 上，这里记住的是「用户选的是
    /// 跟随系统还是指定明暗」，以及用来在设置页上显示当前选项。
    appearance: AppearanceMode,
    settings_feedback: Option<SharedString>,

    /// 控件订阅。持有它们本身就是目的：条目在，订阅才活着。
    _subscriptions: Vec<Subscription>,
    /// 文字行的订阅单独放：载入预设会整批换掉这些行，旧订阅必须跟着一起走。
    text_line_subscriptions: Vec<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let params = WatermarkParams::default();
        let text = Text::default();

        let preview = cx.new(|_| WatermarkPreview::new());
        let aspect_choice = aspect_choice_for(&params);
        let (controls, subscriptions) = ParameterControls::new(
            &params,
            &text.time_format,
            text.position,
            &aspect_choice,
            window,
            cx,
        );

        let font_names = window
            .text_system()
            .all_font_names()
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>();

        let (text_lines, text_line_subscriptions) =
            build_text_lines(&text, &font_names, window, cx);

        let preset_names = config::list_presets()
            .unwrap_or_default()
            .into_iter()
            .map(SharedString::from)
            .collect();

        let settings = config::load_settings();

        // 系统在明暗之间切换时通知一次；只有「跟随系统」才需要响应。
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if this.appearance == AppearanceMode::System {
                Theme::sync_system_appearance(Some(window), cx);
                cx.notify();
            }
        }));

        let view = Self {
            page: AppPage::Watermark,
            photos: Vec::new(),
            selected: None,
            next_photo_id: 0,
            params,
            text_position: text.position,
            time_format: text.time_format.clone(),
            text_lines,
            next_text_line_id: text.template.len() as u64,
            font_names,
            expanded_text_lines: Vec::new(),
            border_width_open: false,
            aspect_choice,
            controls,
            preview,
            export: ExportState::Idle,
            preset_names,
            preset_feedback: None,
            preset_feedback_is_error: false,
            appearance: settings.appearance,
            settings_feedback: None,
            _subscriptions: subscriptions,
            text_line_subscriptions,
        };
        // 主题要在第一帧之前落好，否则会先闪一下默认的浅色。
        view.apply_appearance(window, cx);
        view
    }

    // MARK: 明暗外观

    /// 把当前的明暗选择落到主题和窗口外观上。
    ///
    /// 窗口外观要单独设：GPUI 的原生窗口边框和标题栏由 `NSApplication.appearance` 决定，
    /// 它不会跟着 GPUI 的主题走，所以「应用是深色而系统是浅色」时窗口边缘会不匹配。
    fn apply_appearance(&self, window: &mut Window, cx: &mut App) {
        match self.appearance {
            AppearanceMode::System => {
                // 先清掉可能存在的强制外观，否则窗口会一直停在上一次的选择上。
                cx.set_window_appearance(None);
                Theme::sync_system_appearance(Some(window), cx);
            }
            AppearanceMode::Light => {
                cx.set_window_appearance(Some(WindowAppearance::Light));
                Theme::change(ThemeMode::Light, Some(window), cx);
            }
            AppearanceMode::Dark => {
                cx.set_window_appearance(Some(WindowAppearance::Dark));
                Theme::change(ThemeMode::Dark, Some(window), cx);
            }
        }
    }

    /// 当前的明暗选择。
    pub fn appearance(&self) -> AppearanceMode {
        self.appearance
    }

    pub(super) fn set_appearance_mode(
        &mut self,
        mode: AppearanceMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.appearance == mode {
            return;
        }
        self.appearance = mode;
        self.apply_appearance(window, cx);

        // 存不下来不影响这次使用，但下次启动不会记住，得让人知道。
        self.settings_feedback = config::save_settings(&config::AppSettings { appearance: mode })
            .err()
            .map(|error| format!("偏好没能保存：{error:#}").into());

        cx.notify();
    }

    // MARK: 读取

    /// 队列里的照片数量。
    pub fn photo_count(&self) -> usize {
        self.photos.len()
    }

    /// 当前选中的照片路径。
    pub fn selected_path(&self) -> Option<&std::path::Path> {
        self.selected_photo().map(|photo| photo.path.as_path())
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
        let id = self.selected?;
        self.photos.iter().find(|photo| photo.id == id)
    }

    fn photo_mut(&mut self, id: PhotoId) -> Option<&mut QueuedPhoto> {
        self.photos.iter_mut().find(|photo| photo.id == id)
    }

    /// 由控件状态拼出渲染用的 [`Text`]。
    ///
    /// 控件实体是文字水印的真值来源，[`Text`] 只是它们在渲染管线里的投影，因此不需要在
    /// 每次回调里手工同步两份数据。
    pub(super) fn build_text(&self, cx: &App) -> Text {
        Text {
            template: self
                .text_lines
                .iter()
                .map(|line| line.template.read(cx).value().to_string())
                .collect(),
            text_params: self
                .text_lines
                .iter()
                .map(|line| TextParams {
                    font: line.font_name(cx),
                    size: line.size.value(cx),
                    line_spacing: line.line_spacing.value(cx),
                    color: if line.auto_color {
                        None
                    } else {
                        Some(line.color.value(cx))
                    },
                    italic: line.italic,
                    bold: line.bold,
                    align: line.alignment(cx),
                    ..TextParams::default()
                })
                .collect(),
            position: self.text_position,
            time_format: self.time_format.clone(),
        }
    }

    // MARK: 预览

    /// 参数、选中项或文字水印变化后调用：把当前状态打包成一份预览请求。
    ///
    /// 请求本身只是「登记最新意图」，真正算不算、什么时候算由 [`WatermarkPreview`] 决定，
    /// 所以拖动滑块时可以放心地每帧调用。
    pub(super) fn refresh_preview(&self, cx: &mut Context<Self>) {
        let job = match self.selected_photo() {
            Some(photo) => PreviewJob {
                path: photo.path.clone(),
                exif: photo.exif.clone(),
                params: self.params.clone(),
                text: self.build_text(cx),
            },
            None => {
                self.preview.update(cx, |preview, cx| preview.clear(cx));
                return;
            }
        };
        self.preview
            .update(cx, |preview, cx| preview.request(job, cx));
    }

    // MARK: 页面与照片

    pub fn go_to(&mut self, page: AppPage, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            cx.notify();
        }
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
            if self.photos.iter().any(|photo| photo.path == path) {
                continue;
            }
            let id = PhotoId(self.next_photo_id);
            self.next_photo_id += 1;
            self.photos.push(QueuedPhoto {
                id,
                path: path.clone(),
                exif: None,
                thumbnail: Thumbnail::Pending,
            });
            added.push((id, path));
        }

        if added.is_empty() {
            return;
        }

        let first = added[0].0;
        for (id, path) in added {
            self.load_thumbnail(id, path.clone(), cx);
            self.load_exif(id, path, cx);
        }

        if self.selected.is_none() {
            self.selected = Some(first);
            self.refresh_preview(cx);
        }
        cx.notify();
    }

    pub(super) fn select_photo(&mut self, id: PhotoId, cx: &mut Context<Self>) {
        if self.selected == Some(id) {
            return;
        }
        self.selected = Some(id);
        self.refresh_preview(cx);
        cx.notify();
    }

    /// 按队列顺序选中第 `index` 张照片。
    ///
    /// 选中项本身由领域 id 标识；这里是按位置操作队列的一条受控通道。
    pub fn select_photo_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(id) = self.photos.get(index).map(|photo| photo.id) else {
            return;
        };
        self.select_photo(id, cx);
    }

    pub fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        let Some(ix) = self.photos.iter().position(|photo| photo.id == id) else {
            return;
        };
        self.photos.remove(ix);
        // 选中项落到原位置的下一张，没有就退回上一张：删掉一张后视线不必重新找位置。
        self.selected = self
            .photos
            .get(ix)
            .or_else(|| self.photos.last())
            .map(|photo| photo.id);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub fn clear_photos(&mut self, cx: &mut Context<Self>) {
        if self.photos.is_empty() {
            return;
        }
        self.photos.clear();
        self.selected = None;
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
                let Some(photo) = this.photo_mut(id) else {
                    return;
                };
                photo.thumbnail = match rendered {
                    Ok(image) => Thumbnail::Ready(image),
                    Err(_) => Thumbnail::Failed,
                };
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
                let is_selected = this.selected == Some(id);
                if let Some(photo) = this.photo_mut(id) {
                    photo.exif = exif;
                }
                if is_selected {
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
    pub(in crate::ui) fn apply_aspect_choice(&mut self, choice: AspectRatioChoice, cx: &App) {
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

    // MARK: 文字水印行

    pub(super) fn add_text_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_text_line_id;
        self.next_text_line_id += 1;

        // 新行沿用上一行的样式：连续添加几行时不用逐行重设字号和对齐。
        let inherited = self
            .text_lines
            .last()
            .map(|last| TextParams {
                font: last.font_name(cx),
                size: last.size.value(cx),
                line_spacing: last.line_spacing.value(cx),
                color: if last.auto_color {
                    None
                } else {
                    Some(last.color.value(cx))
                },
                italic: last.italic,
                bold: last.bold,
                align: last.alignment(cx),
                ..TextParams::default()
            })
            .unwrap_or_default();

        let (line, subscriptions) = TextLine::new(id, "", &inherited, &self.font_names, window, cx);
        self.text_lines.push(line);
        self.text_line_subscriptions.extend(subscriptions);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(super) fn remove_text_line(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.text_lines.iter().position(|line| line.id == id) else {
            return;
        };
        // 行自己的订阅由它自己的控件持有，随实体一起消失；这里额外检查一下集合是否
        // 还和行对得上，避免留下指向已删行的回调。
        self.text_lines.remove(ix);
        self.expanded_text_lines.retain(|open| *open != id);
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(super) fn set_text_line_auto_color(
        &mut self,
        id: u64,
        auto_color: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(line) = self.text_lines.iter_mut().find(|line| line.id == id) {
            line.auto_color = auto_color;
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    pub(super) fn set_text_line_style(
        &mut self,
        id: u64,
        bold: Option<bool>,
        italic: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if let Some(line) = self.text_lines.iter_mut().find(|line| line.id == id) {
            if let Some(bold) = bold {
                line.bold = bold;
            }
            if let Some(italic) = italic {
                line.italic = italic;
            }
        }
        self.refresh_preview(cx);
        cx.notify();
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

        let preset = preset_of(&self.params, &self.build_text(cx));
        match config::save_preset(&name, &preset) {
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
        match config::load_preset(name) {
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
        // 输出文件夹是这台机器的环境设置，不跟着预设走。
        let output_folder = self.params.output_folder.clone();
        self.params = preset.params;
        self.params.output_folder = output_folder;

        self.text_position = preset.text.position;
        self.time_format = preset.text.time_format.clone();

        // 文字行整批重建：行数、模板和每行控件都要对上这份配置。
        let (lines, subscriptions) = build_text_lines(&preset.text, &self.font_names, window, cx);
        self.text_lines = lines;
        self.text_line_subscriptions = subscriptions;
        self.expanded_text_lines.clear();

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
        let text_position = self.text_position;
        controls.text_position.update(cx, |state, cx| {
            state.set_selected_value(&text_position, window, cx)
        });

        let time_format = self.time_format.clone();
        controls
            .time_format
            .update(cx, |state, cx| state.set_value(time_format, window, cx));

        // 自定义比例的两个框：不在自定义模式时也同步，切过去就能直接用。
        let (width, height) = params.aspect_ratio.unwrap_or((1.0, 1.0));
        controls.aspect_width.update(cx, |state, cx| {
            state.set_value(format_ratio(width), window, cx)
        });
        controls.aspect_height.update(cx, |state, cx| {
            state.set_value(format_ratio(height), window, cx)
        });
    }

    /// 弹一个确认框再删预设：文件删掉就找不回来了。
    pub(super) fn confirm_delete_preset(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        match config::delete_preset(name) {
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

    fn refresh_preset_names(&mut self) {
        self.preset_names = config::list_presets()
            .unwrap_or_default()
            .into_iter()
            .map(SharedString::from)
            .collect();
    }

    // MARK: 导出

    /// 按当前参数把队列里的照片全部导出。
    ///
    /// 逐张串行处理而不是并发：libvips 自己就吃满多核，再叠并发只会让每张都变慢，还会
    /// 让进度读数失去意义。
    pub(super) fn export_all(&mut self, cx: &mut Context<Self>) {
        if self.photos.is_empty() || matches!(self.export, ExportState::Running { .. }) {
            return;
        }
        if self.params.output_folder.is_none() {
            return;
        }

        let params = self.params.clone();
        let text = self.build_text(cx);
        let paths: Vec<PathBuf> = self.photos.iter().map(|photo| photo.path.clone()).collect();
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
                let text = text.clone();
                let result = cx
                    .background_spawn(async move {
                        let photo = Photo::open_blocking(&path)?;
                        let watermark = photo.generate_watermark(&params, &text)?;
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

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (center, inspector) = match self.page {
            AppPage::Watermark => (
                self.render_watermark_page(cx).into_any_element(),
                Some(self.render_inspector(cx).into_any_element()),
            ),
            AppPage::Settings => (self.render_settings_page(cx).into_any_element(), None),
        };

        h_flex()
            .items_stretch()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_sidebar(window, cx))
            .child(center)
            .children(inspector)
            // 叠加层必须由应用的第一个视图渲染一次，`Root` 只负责协调它们。
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

impl AppView {
    /// 水印工作区：上方预览、下方队列。
    fn render_watermark_page(&self, cx: &Context<Self>) -> impl IntoElement {
        // 拖放挂在整块工作区上：拖到预览上也算数，不必瞄准下面那条队列。
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_preview_pane(cx))
                    .child(self.render_queue_pane(cx)),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_photos(paths.paths().to_vec(), cx)
            }))
    }
}

/// 按一份 [`Text`] 建出全部文字行。
fn build_text_lines(
    text: &Text,
    fonts: &[SharedString],
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> (Vec<TextLine>, Vec<Subscription>) {
    let mut lines = Vec::new();
    let mut subscriptions = Vec::new();

    for (ix, (template, params)) in text
        .template
        .iter()
        .zip(text.text_params.iter())
        .enumerate()
    {
        let (line, line_subscriptions) =
            TextLine::new(ix as u64, template, params, fonts, window, cx);
        lines.push(line);
        subscriptions.extend(line_subscriptions);
    }
    (lines, subscriptions)
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
