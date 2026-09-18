//! 小工具页面：彩色恢复 Gain Map 与从视频生成 Motion Photo。

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, ExternalPaths, FontWeight, ObjectFit, div, img};

use super::super::AppView;
use super::super::component::colour_gainmap_preview::{
    ColourGainMapPreview, ColourGainMapPreviewState,
};
use super::super::component::field::rgb_to_hsla;
use super::super::component::motion_photo_preview::{MotionPhotoPreview, MotionPhotoPreviewState};
use crate::process::motion_photo::inspect_motion_photo_video;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::app) enum UtilityMode {
    ColourGainMap,
    MotionPhoto,
}

#[derive(Clone, Copy)]
pub(in crate::ui::app) enum MotionTimeControl {
    Start,
    End,
    Cover,
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
}

impl ColourGainMapPageState {
    pub(in crate::ui::app) fn new(cx: &mut Context<AppView>) -> Self {
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

    pub(in crate::ui::app) fn select(&mut self, path: PathBuf, cx: &mut Context<AppView>) {
        self.path = Some(path.clone());
        self.preview
            .update(cx, |preview, cx| preview.request(path, cx));
    }

    pub(in crate::ui::app) fn select_motion_video(
        &mut self,
        path: PathBuf,
        cx: &mut Context<AppView>,
    ) -> anyhow::Result<()> {
        let info = inspect_motion_photo_video(&path)?;
        let end = info.duration.min(Duration::from_secs(10));
        if end.is_zero() {
            anyhow::bail!("视频时长为 0，无法生成实况照片");
        }
        self.video_path = Some(path);
        self.video_duration = Some(info.duration);
        self.motion_start = Duration::ZERO;
        self.motion_end = end;
        self.motion_cover = end / 2;
        self.request_motion_preview(cx);
        cx.notify();
        Ok(())
    }

    pub(in crate::ui::app) fn adjust_motion_time(
        &mut self,
        control: MotionTimeControl,
        delta: f64,
        cx: &mut Context<AppView>,
    ) {
        let max = self
            .video_duration
            .unwrap_or(Duration::from_secs(10))
            .min(Duration::from_secs(10));
        if max <= Duration::from_millis(100) {
            return;
        }
        let value = |time: Duration| (time.as_secs_f64() + delta).clamp(0.0, max.as_secs_f64());
        match control {
            MotionTimeControl::Start => {
                self.motion_start = Duration::from_secs_f64(
                    value(self.motion_start)
                        .min((self.motion_end - Duration::from_millis(100)).as_secs_f64()),
                );
            }
            MotionTimeControl::End => {
                self.motion_end = Duration::from_secs_f64(
                    value(self.motion_end)
                        .max((self.motion_start + Duration::from_millis(100)).as_secs_f64()),
                );
            }
            MotionTimeControl::Cover => {
                self.motion_cover = Duration::from_secs_f64(value(self.motion_cover).clamp(
                    self.motion_start.as_secs_f64(),
                    (self.motion_end - Duration::from_micros(1)).as_secs_f64(),
                ));
            }
        }
        self.motion_cover = self.motion_cover.clamp(
            self.motion_start,
            self.motion_end - Duration::from_micros(1),
        );
        self.request_motion_preview(cx);
        cx.notify();
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
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if this.colour_gainmap.mode() == UtilityMode::ColourGainMap {
                    this.add_colour_gainmap_photo(paths.paths().to_vec(), cx);
                } else {
                    this.add_motion_photo_video(paths.paths().to_vec(), cx);
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
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("motion-photo-open")
                            .label("选择视频…")
                            .small()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.pick_motion_photo_video(cx)),
                            ),
                    )
                    .child(
                        Button::new("motion-photo-export")
                            .label(if self.colour_gainmap.is_motion_exporting() {
                                "正在导出…"
                            } else {
                                "导出 Motion Photo…"
                            })
                            .small()
                            .disabled(
                                self.colour_gainmap.video_path().is_none()
                                    || self.colour_gainmap.is_motion_exporting(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_motion_photo(window, cx)
                            })),
                    )
                    .into_any_element()
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
            ColourGainMapPreviewState::Empty => placeholder(IconName::Palette, "选择或拖入一张照片", cx),
            ColourGainMapPreviewState::Loading => placeholder(IconName::Loader, "正在生成预览…", cx),
            ColourGainMapPreviewState::Failed(message) => div().text_color(cx.theme().danger).child(message.clone()).into_any_element(),
            ColourGainMapPreviewState::Ready { black_and_white, .. } => img(black_and_white.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element(),
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
            .min_h_0()
            .gap_5()
            .p_5()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .p_3()
                    .gap_2()
                    .bg(rgb_to_hsla(self.preview_background))
                    .rounded_md()
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
                    .w_72()
                    .flex_shrink_0()
                    .gap_4()
                    .p_4()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_md()
                    .child(div().font_weight(FontWeight::MEDIUM).child("视频片段"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.motion_video_summary()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("仅支持 MP4 容器、H.264/AVC 视频；导出的动态片段最长 10 秒。"),
                    )
                    .child(self.render_motion_time_control(
                        "入点",
                        MotionTimeControl::Start,
                        self.colour_gainmap.motion_start(),
                        cx,
                    ))
                    .child(self.render_motion_time_control(
                        "出点",
                        MotionTimeControl::End,
                        self.colour_gainmap.motion_end(),
                        cx,
                    ))
                    .child(self.render_motion_time_control(
                        "封面",
                        MotionTimeControl::Cover,
                        self.colour_gainmap.motion_cover(),
                        cx,
                    )),
            )
    }

    fn render_motion_time_control(
        &self,
        label: &'static str,
        control: MotionTimeControl,
        value: Duration,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let index = match control {
            MotionTimeControl::Start => 0_u64,
            MotionTimeControl::End => 1_u64,
            MotionTimeControl::Cover => 2_u64,
        };
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .child(div().text_sm().child(label))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new(("motion-time-minus", index))
                            .label("−")
                            .small()
                            .ghost()
                            .disabled(self.colour_gainmap.video_path().is_none())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.colour_gainmap.adjust_motion_time(control, -0.25, cx)
                            })),
                    )
                    .child(
                        div()
                            .w_16()
                            .text_center()
                            .text_sm()
                            .child(format!("{:.2} 秒", value.as_secs_f64())),
                    )
                    .child(
                        Button::new(("motion-time-plus", index))
                            .label("+")
                            .small()
                            .ghost()
                            .disabled(self.colour_gainmap.video_path().is_none())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.colour_gainmap.adjust_motion_time(control, 0.25, cx)
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
