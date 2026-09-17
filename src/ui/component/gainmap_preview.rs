//! HDR gain map 的后台解析与展示状态。

use std::{path::PathBuf, sync::Arc};

use gpui_kit::{AppContext as _, Context, RenderImage, SharedString, Task};

use crate::ui::image::render_gainmap_preview;

pub enum GainMapPreviewState {
    Empty,
    Loading,
    Ready(Arc<RenderImage>),
    Missing,
    Failed(SharedString),
}

pub struct GainMapPreview {
    state: GainMapPreviewState,
    generation: u64,
    worker: Option<Task<()>>,
}

impl GainMapPreview {
    pub fn new() -> Self {
        Self {
            state: GainMapPreviewState::Empty,
            generation: 0,
            worker: None,
        }
    }

    pub fn state(&self) -> &GainMapPreviewState {
        &self.state
    }

    pub fn request(&mut self, path: PathBuf, show_gainmap: bool, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.state = GainMapPreviewState::Loading;
        cx.notify();
        self.worker = Some(cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_spawn(async move { render_gainmap_preview(&path, show_gainmap) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.state = match rendered {
                    Ok(Some(image)) => GainMapPreviewState::Ready(image),
                    Ok(None) => GainMapPreviewState::Missing,
                    Err(error) => GainMapPreviewState::Failed(format!("{error:#}").into()),
                };
                cx.notify();
            });
        }));
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.state = GainMapPreviewState::Empty;
        cx.notify();
    }
}
