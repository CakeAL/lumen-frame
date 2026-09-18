//! 彩色恢复 Gain Map 的后台预览状态。

use std::{path::PathBuf, sync::Arc};

use gpui_kit::{AppContext as _, Context, RenderImage, SharedString, Task};

use crate::ui::image::render_colour_gainmap_preview;

pub enum ColourGainMapPreviewState {
    Empty,
    Loading,
    Ready {
        black_and_white: Arc<RenderImage>,
        gainmap: Arc<RenderImage>,
    },
    Failed(SharedString),
}

pub struct ColourGainMapPreview {
    state: ColourGainMapPreviewState,
    generation: u64,
    worker: Option<Task<()>>,
}

impl ColourGainMapPreview {
    pub fn new() -> Self {
        Self {
            state: ColourGainMapPreviewState::Empty,
            generation: 0,
            worker: None,
        }
    }

    pub fn state(&self) -> &ColourGainMapPreviewState {
        &self.state
    }

    pub fn request(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.state = ColourGainMapPreviewState::Loading;
        cx.notify();
        self.worker = Some(cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_spawn(async move { render_colour_gainmap_preview(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.state = match rendered {
                        Ok((black_and_white, gainmap)) => ColourGainMapPreviewState::Ready {
                            black_and_white,
                            gainmap,
                        },
                        Err(error) => {
                            ColourGainMapPreviewState::Failed(format!("{error:#}").into())
                        }
                    };
                    cx.notify();
                }
            });
        }));
    }
}
