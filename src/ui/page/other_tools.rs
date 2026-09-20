//! 小工具页面：彩色恢复 Gain Map 与从视频生成 Motion Photo。

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    scroll::ScrollableElement as _,
    slider::{Slider, SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, ExternalPaths, FontWeight, ObjectFit, Subscription, Window,
    div, img,
};

use super::super::AppView;
use super::super::component::colour_gainmap_preview::{
    ColourGainMapPreview, ColourGainMapPreviewState,
};
use super::super::component::field::rgb_to_hsla;
use super::super::component::motion_photo_preview::{MotionPhotoPreview, MotionPhotoPreviewState};
use crate::process::motion_photo::{
    DEFAULT_MAX_OUTPUT_SIZE, detect_ffmpeg, inspect_motion_photo_video, validate_ffmpeg,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::app) enum UtilityMode {
    ColourGainMap,
    MotionPhoto,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::app) enum MotionTimeControl {
    Start,
    End,
    Cover,
}

const MAX_MOTION_DURATION: Duration = Duration::from_secs(10);
const MIN_MOTION_DURATION: Duration = Duration::from_millis(100);

/// 时间参数的两条输入路径：滑块快速定位，输入框精确填写秒数。
///
/// 滑块内部保存 0..=1 的归一化位置，因此载入任意时长的视频后都无需重建控件状态。
struct MotionTimeField {
    slider: Entity<SliderState>,
    input: Entity<InputState>,
}

impl MotionTimeField {
    fn new(
        initial_seconds: f64,
        initial_timeline: f64,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let normalized = if initial_timeline > 0.0 {
            initial_seconds / initial_timeline
        } else {
            0.0
        };
        Self {
            slider: cx.new(|_| {
                SliderState::new()
                    .max(1.0)
                    .min(0.0)
                    .step(0.000_1)
                    .default_value(normalized.clamp(0.0, 1.0) as f32)
            }),
            input: cx.new(|cx| {
                InputState::new(window, cx).default_value(format!("{initial_seconds:.2}"))
            }),
        }
    }

    fn subscribe(
        &self,
        control: MotionTimeControl,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Vec<Subscription> {
        let slider_subscription = cx.subscribe_in(
            &self.slider,
            window,
            move |this, _, event, window, cx| match event {
                SliderEvent::Change(value) => {
                    let timeline = this
                        .colour_gainmap
                        .video_duration()
                        .unwrap_or(MAX_MOTION_DURATION)
                        .as_secs_f64();
                    this.colour_gainmap.set_motion_time(
                        control,
                        f64::from(value.end()) * timeline,
                        None,
                        window,
                        cx,
                    );
                }
                // 拖动过程中只更新数值；松手后才解码一次封面，避免连续启动 FFmpeg。
                SliderEvent::Release(_) => this.colour_gainmap.request_motion_preview(cx),
            },
        );

        let input_subscription = cx.subscribe_in(
            &self.input,
            window,
            move |this, state, event, window, cx| match event {
                InputEvent::Change => {
                    let Some(seconds) = parse_seconds(&state.read(cx).value()) else {
                        return;
                    };
                    this.colour_gainmap.set_motion_time(
                        control,
                        seconds,
                        Some(control),
                        window,
                        cx,
                    );
                }
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    if let Some(seconds) = parse_seconds(&state.read(cx).value()) {
                        this.colour_gainmap
                            .set_motion_time(control, seconds, None, window, cx);
                    } else {
                        this.colour_gainmap
                            .sync_motion_time_fields(None, window, cx);
                    }
                    this.colour_gainmap.request_motion_preview(cx);
                }
                InputEvent::Focus => {}
            },
        );

        vec![slider_subscription, input_subscription]
    }

    fn sync(
        &self,
        seconds: f64,
        timeline: f64,
        sync_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let normalized = if timeline > 0.0 {
            (seconds / timeline).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if (f64::from(self.slider.read(cx).value().end()) - normalized).abs() > 1e-6 {
            self.slider.update(cx, |state, cx| {
                state.set_value(normalized as f32, window, cx)
            });
        }
        if sync_input {
            write_input_seconds(&self.input, seconds, window, cx);
        }
    }

    fn render(&self, label: &'static str, disabled: bool, cx: &App) -> AnyElement {
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(div().text_sm().child(label))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Input::new(&self.input).small().w_20().disabled(disabled))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("秒"),
                            ),
                    ),
            )
            .child(Slider::new(&self.slider).disabled(disabled).w_full())
            .into_any_element()
    }
}

pub(in crate::ui::app) struct ColourGainMapPageState {
    mode: UtilityMode,
    path: Option<PathBuf>,
    preview: Entity<ColourGainMapPreview>,
    exporting: bool,
    video_path: Option<PathBuf>,
    video_duration: Option<Duration>,
    motion_start: Duration,
    motion_end: Duration,
    motion_cover: Duration,
    motion_preview: Entity<MotionPhotoPreview>,
    motion_exporting: bool,
    motion_max_size_mb: u32,
    ffmpeg_path: Option<PathBuf>,
    ffmpeg_input: Entity<InputState>,
    motion_start_field: MotionTimeField,
    motion_end_field: MotionTimeField,
    motion_cover_field: MotionTimeField,
    _motion_time_subscriptions: Vec<Subscription>,
}

impl ColourGainMapPageState {
    pub(in crate::ui::app) fn new(
        window: &mut gpui_kit::Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let ffmpeg_path = detect_ffmpeg().ok();
        let ffmpeg_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(
                    ffmpeg_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                )
                .placeholder("FFmpeg 可执行文件路径")
        });
        let motion_start_field = MotionTimeField::new(0.0, 10.0, window, cx);
        let motion_end_field = MotionTimeField::new(10.0, 10.0, window, cx);
        let motion_cover_field = MotionTimeField::new(5.0, 10.0, window, cx);
        let mut motion_time_subscriptions = Vec::new();
        motion_time_subscriptions.extend(motion_start_field.subscribe(
            MotionTimeControl::Start,
            window,
            cx,
        ));
        motion_time_subscriptions.extend(motion_end_field.subscribe(
            MotionTimeControl::End,
            window,
            cx,
        ));
        motion_time_subscriptions.extend(motion_cover_field.subscribe(
            MotionTimeControl::Cover,
            window,
            cx,
        ));
        Self {
            mode: UtilityMode::ColourGainMap,
            path: None,
            preview: cx.new(|_| ColourGainMapPreview::new()),
            exporting: false,
            video_path: None,
            video_duration: None,
            motion_start: Duration::ZERO,
            motion_end: Duration::from_secs(10),
            motion_cover: Duration::from_secs(5),
            motion_preview: cx.new(|_| MotionPhotoPreview::new()),
            motion_exporting: false,
            motion_max_size_mb: (DEFAULT_MAX_OUTPUT_SIZE / 1024 / 1024) as u32,
            ffmpeg_path,
            ffmpeg_input,
            motion_start_field,
            motion_end_field,
            motion_cover_field,
            _motion_time_subscriptions: motion_time_subscriptions,
        }
    }

    pub(in crate::ui::app) fn mode(&self) -> UtilityMode {
        self.mode
    }
    pub(in crate::ui::app) fn set_mode(&mut self, mode: UtilityMode, cx: &mut Context<AppView>) {
        if self.mode != mode {
            self.mode = mode;
            cx.notify();
        }
    }
    pub(in crate::ui::app) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub(in crate::ui::app) fn preview(&self) -> &Entity<ColourGainMapPreview> {
        &self.preview
    }
    pub(in crate::ui::app) fn is_exporting(&self) -> bool {
        self.exporting
    }
    pub(in crate::ui::app) fn video_path(&self) -> Option<&Path> {
        self.video_path.as_deref()
    }
    pub(in crate::ui::app) fn video_duration(&self) -> Option<Duration> {
        self.video_duration
    }
    pub(in crate::ui::app) fn motion_start(&self) -> Duration {
        self.motion_start
    }
    pub(in crate::ui::app) fn motion_end(&self) -> Duration {
        self.motion_end
    }
    pub(in crate::ui::app) fn motion_cover(&self) -> Duration {
        self.motion_cover
    }
    pub(in crate::ui::app) fn motion_preview(&self) -> &Entity<MotionPhotoPreview> {
        &self.motion_preview
    }
    pub(in crate::ui::app) fn is_motion_exporting(&self) -> bool {
        self.motion_exporting
    }
    pub(in crate::ui::app) fn motion_max_size_bytes(&self) -> u64 {
        self.motion_max_size_mb as u64 * 1024 * 1024
    }
    pub(in crate::ui::app) fn motion_max_size_mb(&self) -> u32 {
        self.motion_max_size_mb
    }
    pub(in crate::ui::app) fn adjust_motion_max_size(
        &mut self,
        delta: i32,
        cx: &mut Context<AppView>,
    ) {
        self.motion_max_size_mb = (self.motion_max_size_mb as i32 + delta).clamp(1, 1024) as u32;
        cx.notify();
    }
    pub(in crate::ui::app) fn ffmpeg_path(&self) -> Option<&Path> {
        self.ffmpeg_path.as_deref()
    }
    pub(in crate::ui::app) fn ffmpeg_input(&self) -> &Entity<InputState> {
        &self.ffmpeg_input
    }
    pub(in crate::ui::app) fn detect_ffmpeg(
        &mut self,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> anyhow::Result<()> {
        self.ffmpeg_path = Some(detect_ffmpeg()?);
        self.sync_ffmpeg_input(window, cx);
        cx.notify();
        Ok(())
    }
    pub(in crate::ui::app) fn set_ffmpeg_path(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> anyhow::Result<()> {
        validate_ffmpeg(&path)?;
        self.ffmpeg_path = Some(path);
        self.sync_ffmpeg_input(window, cx);
        cx.notify();
        Ok(())
    }
    pub(in crate::ui::app) fn apply_ffmpeg_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> anyhow::Result<()> {
        let value = self.ffmpeg_input.read(cx).value().trim().to_owned();
        anyhow::ensure!(!value.is_empty(), "请填写 FFmpeg 可执行文件路径");
        self.set_ffmpeg_path(PathBuf::from(value), window, cx)
    }

    fn sync_ffmpeg_input(&self, window: &mut Window, cx: &mut App) {
        let Some(path) = self.ffmpeg_path.as_ref() else {
            return;
        };
        let value = path.display().to_string();
        if self.ffmpeg_input.read(cx).value().as_ref() != value {
            self.ffmpeg_input
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    pub(in crate::ui::app) fn select(&mut self, path: PathBuf, cx: &mut Context<AppView>) {
        self.path = Some(path.clone());
        self.preview
            .update(cx, |preview, cx| preview.request(path, cx));
    }

    pub(in crate::ui::app) fn select_motion_video(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> anyhow::Result<()> {
        let ffmpeg = self.ffmpeg_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!("未找到 FFmpeg。请自动检测或手动选择 FFmpeg 可执行文件")
        })?;
        let info = inspect_motion_photo_video(ffmpeg, &path)?;
        let end = info.duration.min(MAX_MOTION_DURATION);
        if end.is_zero() {
            anyhow::bail!("视频时长为 0，无法生成实况照片");
        }
        self.video_path = Some(path);
        self.video_duration = Some(info.duration);
        self.motion_start = Duration::ZERO;
        self.motion_end = end;
        self.motion_cover = end / 2;
        self.sync_motion_time_fields(None, window, cx);
        self.request_motion_preview(cx);
        cx.notify();
        Ok(())
    }

    fn set_motion_time(
        &mut self,
        control: MotionTimeControl,
        seconds: f64,
        source_input: Option<MotionTimeControl>,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        let timeline = self.video_duration.unwrap_or(MAX_MOTION_DURATION);
        if timeline.is_zero() {
            return;
        }
        (self.motion_start, self.motion_end, self.motion_cover) = updated_motion_times(
            control,
            seconds,
            timeline,
            self.motion_start,
            self.motion_end,
            self.motion_cover,
        );
        self.sync_motion_time_fields(source_input, window, cx);
        cx.notify();
    }

    fn sync_motion_time_fields(
        &self,
        source_input: Option<MotionTimeControl>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let timeline = self
            .video_duration
            .unwrap_or(MAX_MOTION_DURATION)
            .as_secs_f64();
        self.motion_start_field.sync(
            self.motion_start.as_secs_f64(),
            timeline,
            source_input != Some(MotionTimeControl::Start),
            window,
            cx,
        );
        self.motion_end_field.sync(
            self.motion_end.as_secs_f64(),
            timeline,
            source_input != Some(MotionTimeControl::End),
            window,
            cx,
        );
        self.motion_cover_field.sync(
            self.motion_cover.as_secs_f64(),
            timeline,
            source_input != Some(MotionTimeControl::Cover),
            window,
            cx,
        );
    }

    pub(in crate::ui::app) fn set_exporting(&mut self, exporting: bool, cx: &mut Context<AppView>) {
        if self.exporting != exporting {
            self.exporting = exporting;
            cx.notify();
        }
    }
    pub(in crate::ui::app) fn set_motion_exporting(
        &mut self,
        exporting: bool,
        cx: &mut Context<AppView>,
    ) {
        if self.motion_exporting != exporting {
            self.motion_exporting = exporting;
            cx.notify();
        }
    }

    fn request_motion_preview(&self, cx: &mut Context<AppView>) {
        let Some(path) = self.video_path.clone() else {
            return;
        };
        self.motion_preview.update(cx, |preview, cx| {
            preview.request(
                self.ffmpeg_path.clone(),
                path,
                self.motion_start,
                self.motion_end,
                self.motion_cover,
                cx,
            )
        });
    }
}

impl AppView {
    pub(in crate::ui::app) fn render_colour_gainmap_page(
        &self,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.render_colour_gainmap_toolbar(cx))
            .child(match self.colour_gainmap.mode() {
                UtilityMode::ColourGainMap => {
                    self.render_colour_gainmap_canvas(cx).into_any_element()
                }
                UtilityMode::MotionPhoto => self.render_motion_photo_canvas(cx).into_any_element(),
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if this.colour_gainmap.mode() == UtilityMode::ColourGainMap {
                    this.add_colour_gainmap_photo(paths.paths().to_vec(), cx);
                } else {
                    this.add_motion_photo_video(paths.paths().to_vec(), window, cx);
                }
            }))
    }

    fn render_colour_gainmap_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let motion = self.colour_gainmap.mode() == UtilityMode::MotionPhoto;
        h_flex()
            .w_full()
            .h_12()
            .flex_shrink_0()
            .justify_between()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("小工具"),
                    )
                    .child(
                        Button::new("utility-colour-mode")
                            .label("生成黑白 + 彩色 Gain Map")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.colour_gainmap.set_mode(UtilityMode::ColourGainMap, cx)
                            })),
                    )
                    .child(
                        Button::new("utility-motion-mode")
                            .label("从视频生成实况照片")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.colour_gainmap.set_mode(UtilityMode::MotionPhoto, cx)
                            })),
                    ),
            )
            .child(if motion {
                div().into_any_element()
            } else {
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("colour-gainmap-open")
                            .label("选择照片…")
                            .small()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.pick_colour_gainmap_photo(cx)),
                            ),
                    )
                    .child(
                        Button::new("colour-gainmap-export")
                            .label(if self.colour_gainmap.is_exporting() {
                                "正在生成…"
                            } else {
                                "导出 JPEG…"
                            })
                            .small()
                            .disabled(
                                self.colour_gainmap.path().is_none()
                                    || self.colour_gainmap.is_exporting(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_colour_gainmap(window, cx)
                            })),
                    )
                    .into_any_element()
            })
    }

    fn render_colour_gainmap_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_1()
            .min_h_0()
            .p_5()
            .gap_4()
            .bg(rgb_to_hsla(self.preview_background))
            .child(self.render_image_panel("黑白底图", self.render_black_and_white_preview(cx), cx))
            .child(self.render_image_panel("彩色 Gain Map", self.render_colour_preview(cx), cx))
    }

    fn render_image_panel(
        &self,
        title: &'static str,
        content: gpui_kit::AnyElement,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .gap_2()
            .p_3()
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
    }

    fn render_black_and_white_preview(&self, cx: &Context<Self>) -> gpui_kit::AnyElement {
        match self.colour_gainmap.preview().read(cx).state() {
            ColourGainMapPreviewState::Empty => {
                placeholder(IconName::Palette, "选择或拖入一张照片", cx)
            }
            ColourGainMapPreviewState::Loading => {
                placeholder(IconName::Loader, "正在生成预览…", cx)
            }
            ColourGainMapPreviewState::Failed(message) => div()
                .text_color(cx.theme().danger)
                .child(message.clone())
                .into_any_element(),
            ColourGainMapPreviewState::Ready {
                black_and_white, ..
            } => img(black_and_white.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
        }
    }

    fn render_colour_preview(&self, cx: &Context<Self>) -> gpui_kit::AnyElement {
        match self.colour_gainmap.preview().read(cx).state() {
            ColourGainMapPreviewState::Empty => {
                placeholder(IconName::Palette, "生成结果会显示在这里", cx)
            }
            ColourGainMapPreviewState::Loading => {
                placeholder(IconName::Loader, "正在生成预览…", cx)
            }
            ColourGainMapPreviewState::Failed(message) => div()
                .text_color(cx.theme().danger)
                .child(message.clone())
                .into_any_element(),
            ColourGainMapPreviewState::Ready { gainmap, .. } => img(gainmap.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
        }
    }

    fn render_motion_photo_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let preview = match self.colour_gainmap.motion_preview().read(cx).state() {
            MotionPhotoPreviewState::Empty => {
                placeholder(IconName::Palette, "选择 MP4 视频后显示封面帧", cx)
            }
            MotionPhotoPreviewState::Loading => {
                placeholder(IconName::Loader, "正在解码封面帧…", cx)
            }
            MotionPhotoPreviewState::Failed(message) => div()
                .text_color(cx.theme().danger)
                .child(message.clone())
                .into_any_element(),
            MotionPhotoPreviewState::Ready(image) => img(image.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
        };
        h_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .items_stretch()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .p_5()
                    .gap_3()
                    .bg(rgb_to_hsla(self.preview_background))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("封面帧预览"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .items_center()
                            .justify_center()
                            .child(preview),
                    ),
            )
            .child(
                v_flex()
                    .w_96()
                    .h_full()
                    .min_h_0()
                    .flex_shrink_0()
                    .gap_6()
                    .p_5()
                    .overflow_y_scrollbar()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("生成 Motion Photo"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("选择视频片段与封面帧，然后导出为实况照片。"),
                            ),
                    )
                    .child(self.render_motion_ffmpeg_section(cx))
                    .child(
                        v_flex()
                            .w_full()
                            .gap_3()
                            .child(section_title("视频"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.motion_video_summary()),
                            )
                            .child(
                                Button::new("motion-photo-open")
                                    .label("选择视频…")
                                    .small()
                                    .w_full()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick_motion_photo_video(window, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("支持 MP4 容器中的 H.264、HEVC 或 AV1 视频。"),
                            ),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_4()
                            .child(section_title("时间范围"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        "滑块覆盖整段视频，输入框可精确填写秒数；片段最长 10 秒。",
                                    ),
                            )
                            .child(self.colour_gainmap.motion_start_field.render(
                                "入点",
                                self.colour_gainmap.video_path().is_none(),
                                cx,
                            ))
                            .child(self.colour_gainmap.motion_end_field.render(
                                "出点",
                                self.colour_gainmap.video_path().is_none(),
                                cx,
                            ))
                            .child(self.colour_gainmap.motion_cover_field.render(
                                "封面",
                                self.colour_gainmap.video_path().is_none(),
                                cx,
                            )),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_3()
                            .child(section_title("输出"))
                            .child(self.render_motion_size_control(cx))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("FFmpeg 会在编码前按封面与视频的总大小控制码率。"),
                            )
                            .child(
                                Button::new("motion-photo-export")
                                    .label(if self.colour_gainmap.is_motion_exporting() {
                                        "正在导出…"
                                    } else {
                                        "导出 Motion Photo…"
                                    })
                                    .small()
                                    .primary()
                                    .w_full()
                                    .disabled(
                                        self.colour_gainmap.video_path().is_none()
                                            || self.colour_gainmap.ffmpeg_path().is_none()
                                            || self.colour_gainmap.is_motion_exporting(),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.export_motion_photo(window, cx)
                                    })),
                            ),
                    ),
            )
    }

    fn render_motion_ffmpeg_section(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_3()
            .child(section_title("FFmpeg"))
            .child(Input::new(self.colour_gainmap.ffmpeg_input()).small().w_full())
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new("motion-photo-apply-ffmpeg")
                            .label("应用路径")
                            .small()
                            .flex_1()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Err(error) =
                                    this.colour_gainmap.apply_ffmpeg_input(window, cx)
                                {
                                    window.push_notification(
                                        Notification::error(format!(
                                            "无法使用 FFmpeg：{error:#}"
                                        )),
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("motion-photo-detect-ffmpeg")
                            .label("自动检测")
                            .small()
                            .flex_1()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Err(error) = this.colour_gainmap.detect_ffmpeg(window, cx) {
                                    window.push_notification(
                                        Notification::error(format!(
                                            "无法检测 FFmpeg：{error:#}"
                                        )),
                                        cx,
                                    );
                                }
                            })),
                    ),
            )
            .child(
                Button::new("motion-photo-select-ffmpeg")
                    .label("选择 FFmpeg…")
                    .small()
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| this.pick_ffmpeg(window, cx))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        self.colour_gainmap
                            .ffmpeg_path()
                            .map(|path| format!("当前：{}", path.display()))
                            .unwrap_or_else(|| "尚未检测到 FFmpeg".into()),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("需要 FFmpeg、ffprobe 与 libx264。可自动检测终端 PATH，或手动填写可执行文件路径。"),
            )
    }

    fn render_motion_size_control(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .child(div().text_sm().child("最大文件"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("motion-size-minus")
                            .label("−")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.colour_gainmap.adjust_motion_max_size(-1, cx)
                            })),
                    )
                    .child(
                        div()
                            .w_16()
                            .text_center()
                            .text_sm()
                            .child(format!("{} MB", self.colour_gainmap.motion_max_size_mb())),
                    )
                    .child(
                        Button::new("motion-size-plus")
                            .label("+")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.colour_gainmap.adjust_motion_max_size(1, cx)
                            })),
                    ),
            )
    }

    fn motion_video_summary(&self) -> String {
        let name = self
            .colour_gainmap
            .video_path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "尚未选择视频".into());
        match self.colour_gainmap.video_duration() {
            Some(duration) => format!("{name} · {:.2} 秒", duration.as_secs_f64()),
            None => name.into_owned(),
        }
    }
}

fn placeholder(icon: IconName, text: &'static str, cx: &Context<AppView>) -> gpui_kit::AnyElement {
    div()
        .v_flex()
        .items_center()
        .justify_center()
        .gap_3()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon))
        .child(text)
        .into_any_element()
}

fn section_title(title: &'static str) -> impl IntoElement {
    div().text_sm().font_weight(FontWeight::MEDIUM).child(title)
}

fn parse_seconds(text: &str) -> Option<f64> {
    let value = text.trim().parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn write_input_seconds(
    input: &Entity<InputState>,
    seconds: f64,
    window: &mut Window,
    cx: &mut App,
) {
    let value = format!("{seconds:.2}");
    if input.read(cx).value().as_ref() == value {
        return;
    }
    input.update(cx, |state, cx| state.set_value(value, window, cx));
}

fn updated_motion_times(
    control: MotionTimeControl,
    seconds: f64,
    timeline: Duration,
    current_start: Duration,
    current_end: Duration,
    current_cover: Duration,
) -> (Duration, Duration, Duration) {
    let timeline_seconds = timeline.as_secs_f64();
    let minimum_span = MIN_MOTION_DURATION.min(timeline).as_secs_f64();
    let requested = seconds.clamp(0.0, timeline_seconds);
    let mut start = current_start.as_secs_f64();
    let mut end = current_end.as_secs_f64();
    let current_span = (end - start).clamp(minimum_span, MAX_MOTION_DURATION.as_secs_f64());

    match control {
        MotionTimeControl::Start => {
            start = requested.min(timeline_seconds - minimum_span);
            if end < start + minimum_span {
                end = (start + current_span).min(timeline_seconds);
            }
            end = end.min(start + MAX_MOTION_DURATION.as_secs_f64());
        }
        MotionTimeControl::End => {
            end = requested.max(minimum_span);
            if start > end - minimum_span {
                start = (end - current_span).max(0.0);
            }
            start = start.max(end - MAX_MOTION_DURATION.as_secs_f64());
        }
        MotionTimeControl::Cover => {}
    }

    let cover = if matches!(control, MotionTimeControl::Cover) {
        requested
    } else {
        current_cover.as_secs_f64()
    }
    .clamp(start, (end - 0.000_001).max(start));

    (
        Duration::from_secs_f64(start),
        Duration::from_secs_f64(end),
        Duration::from_secs_f64(cover),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_time_controls_can_select_anywhere_in_a_long_video() {
        let (start, end, cover) = updated_motion_times(
            MotionTimeControl::Start,
            100.0,
            Duration::from_secs(216),
            Duration::ZERO,
            Duration::from_secs(10),
            Duration::from_secs(5),
        );

        assert_eq!(start, Duration::from_secs(100));
        assert_eq!(end, Duration::from_secs(110));
        assert_eq!(cover, Duration::from_secs(100));
    }

    #[test]
    fn motion_time_controls_keep_the_clip_within_ten_seconds() {
        let (start, end, cover) = updated_motion_times(
            MotionTimeControl::End,
            42.0,
            Duration::from_secs(216),
            Duration::from_secs(5),
            Duration::from_secs(15),
            Duration::from_secs(10),
        );

        assert_eq!(start, Duration::from_secs(32));
        assert_eq!(end, Duration::from_secs(42));
        assert_eq!(cover, Duration::from_secs(32));
    }

    #[test]
    fn cover_is_clamped_inside_the_selected_clip() {
        let (start, end, cover) = updated_motion_times(
            MotionTimeControl::Cover,
            30.0,
            Duration::from_secs(60),
            Duration::from_secs(20),
            Duration::from_secs(25),
            Duration::from_secs(22),
        );

        assert_eq!(start, Duration::from_secs(20));
        assert_eq!(end, Duration::from_secs(25));
        assert!(cover >= start && cover < end);
    }
}
