//! Motion Photo 封面帧的后台预览状态。

use std::{path::PathBuf, sync::Arc, time::Duration};

use gpui_kit::{AppContext as _, Context, RenderImage, SharedString, Task};

use crate::rotation::Rotation;
use crate::ui::image::render_motion_photo_cover;

pub enum MotionPhotoPreviewState {
    Empty,
    Loading,
    Ready(Arc<RenderImage>),
    Failed(SharedString),
}

pub struct MotionPhotoPreview {
    state: MotionPhotoPreviewState,
    generation: u64,
    worker: Option<Task<()>>,
}

pub struct MotionPhotoPreviewRequest {
    pub ffmpeg_path: Option<PathBuf>,
    pub path: PathBuf,
    pub start: Duration,
    pub end: Duration,
    pub cover_time: Duration,
    pub rotation: Rotation,
}

impl MotionPhotoPreview {
    pub fn new() -> Self {
        Self {
            state: MotionPhotoPreviewState::Empty,
            generation: 0,
            worker: None,
        }
    }

    pub fn state(&self) -> &MotionPhotoPreviewState {
        &self.state
    }

    pub fn request(&mut self, request: MotionPhotoPreviewRequest, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.state = MotionPhotoPreviewState::Loading;
        cx.notify();
        self.worker = Some(cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_spawn(async move {
                    let ffmpeg_path = request
                        .ffmpeg_path
                        .ok_or_else(|| anyhow::anyhow!("未找到 FFmpeg"))?;
                    render_motion_photo_cover(
                        &ffmpeg_path,
                        &request.path,
                        request.start,
                        request.end,
                        request.cover_time,
                        request.rotation,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.state = match rendered {
                        Ok(image) => MotionPhotoPreviewState::Ready(image),
                        Err(error) => MotionPhotoPreviewState::Failed(format!("{error:#}").into()),
                    };
                    cx.notify();
                }
            });
        }));
    }
}
