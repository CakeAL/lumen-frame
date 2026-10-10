//! cartography 的栅格图层适配为 GPUI 位图；坐标仍是 EXIF 编辑器的草稿。

use super::{
    exif_editor::ExifEditor,
    field::{description, warning},
};
use crate::features::geolocation::{Location, MapService, MapViewport, Place};
use cartography::{
    BoxedImageDataRef, ImageData, ImageFeature, ImagesVecLayer, LabelConfig, Map, MapRenderer,
    RenderingState, Rgba, StyleBuilder, Symbol, geo,
};
#[cfg(test)]
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, AvailableSpace, Bounds, Context, Entity, FocusHandle, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point,
    RenderImage, Role, ScrollDelta, ScrollWheelEvent, SharedString, Subscription, Task, WeakEntity,
    Window, canvas, div, img, point, px, size,
};
use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

gpui_kit::actions!(
    gps_map,
    [MapLeft, MapRight, MapUp, MapDown, ZoomIn, ZoomOut]
);

pub(in crate::ui::app) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", MapLeft, Some("GpsMap")),
        KeyBinding::new("right", MapRight, Some("GpsMap")),
        KeyBinding::new("up", MapUp, Some("GpsMap")),
        KeyBinding::new("down", MapDown, Some("GpsMap")),
        KeyBinding::new("+", ZoomIn, Some("GpsMap")),
        KeyBinding::new("-", ZoomOut, Some("GpsMap")),
    ]);
}

struct MapTile {
    bounds: geo::Rect,
    pixels: image::RgbaImage,
    texture: Arc<RenderImage>,
}
impl ImageData for MapTile {
    fn bounding_box(&self) -> geo::Rect {
        self.bounds
    }
    fn pixel_size(&self) -> (u32, u32) {
        self.pixels.dimensions()
    }
    fn rgba_data(&self) -> Vec<u8> {
        self.pixels.as_raw().clone()
    }
}
type TileFeature = ImageFeature<MapTile>;

/// 此适配只接收 ImagesVecLayer；没有矢量几何、标签或背景需要绘制。
#[derive(Default)]
struct TileRenderer(Vec<(Arc<RenderImage>, geo::Rect)>);
impl MapRenderer<TileFeature> for TileRenderer {
    fn draw_image(
        &mut self,
        _: &RenderingState,
        feature: &TileFeature,
        _: ImageFeature<BoxedImageDataRef<'_>>,
        rect: geo::Rect,
    ) {
        self.0.push((feature.image_data().texture.clone(), rect));
    }
    fn draw_point(
        &mut self,
        _: &RenderingState,
        _: &TileFeature,
        _: &geo::Point,
        _: &Symbol<TileFeature>,
    ) {
    }
    fn draw_polygon(
        &mut self,
        _: &RenderingState,
        _: &TileFeature,
        _: &geo::Polygon,
        _: &Symbol<TileFeature>,
    ) {
    }
    fn draw_label(
        &mut self,
        _: &RenderingState,
        _: &TileFeature,
        _: geo::Point,
        _: &LabelConfig<TileFeature>,
    ) {
    }
    fn draw_background(&mut self, _: &Rgba, _: (f64, f64)) {}
}

pub(super) struct LocationPicker {
    focus: FocusHandle,
    editor: WeakEntity<ExifEditor>,
    editor_window: AnyWindowHandle,
    query: Entity<InputState>,
    latitude: Entity<InputState>,
    longitude: Entity<InputState>,
    viewport: MapViewport,
    selected: Option<Location>,
    map: Arc<Map<TileFeature>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    wheel_time: Option<Instant>,
    service: Option<MapService>,
    latest: Arc<AtomicU64>,
    load_task: Option<Task<()>>,
    loading: bool,
    searching: bool,
    search_time: Option<Instant>,
    search_cache: HashMap<String, Vec<Place>>,
    places: Vec<Place>,
    map_feedback: Option<SharedString>,
    feedback: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl LocationPicker {
    pub fn new(
        editor: WeakEntity<ExifEditor>,
        editor_window: AnyWindowHandle,
        location: Option<Location>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_service(
            editor,
            editor_window,
            location,
            MapService::new(),
            window,
            cx,
        )
    }

    pub(super) fn with_service(
        editor: WeakEntity<ExifEditor>,
        editor_window: AnyWindowHandle,
        location: Option<Location>,
        service: anyhow::Result<MapService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索地点，例如 深圳湾公园"));
        let latitude = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("-90 至 90")
                .default_value(
                    location
                        .map(|p| format!("{:.6}", p.latitude()))
                        .unwrap_or_default(),
                )
        });
        let longitude = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("-180 至 180")
                .default_value(
                    location
                        .map(|p| format!("{:.6}", p.longitude()))
                        .unwrap_or_default(),
                )
        });
        let mut subscriptions = vec![cx.subscribe_in(&query, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        })];
        if let Some(owner) = editor.upgrade() {
            subscriptions.push(
                cx.observe_release_in(&owner, window, |_, _, window, _| window.remove_window()),
            );
        }
        let map_feedback = service
            .as_ref()
            .err()
            .map(|_| "地图暂时不可用，仍可手动填写经纬度。".into());
        let mut this = Self {
            focus: cx.focus_handle(),
            editor,
            editor_window,
            query,
            latitude,
            longitude,
            viewport: MapViewport::new(location),
            selected: location,
            map: Arc::new(Map::new()),
            bounds: Rc::new(Cell::new(Bounds::default())),
            drag: None,
            wheel_time: None,
            service: service.ok(),
            latest: Arc::new(AtomicU64::new(0)),
            load_task: None,
            loading: false,
            searching: false,
            search_time: None,
            search_cache: HashMap::new(),
            places: Vec::new(),
            map_feedback,
            feedback: None,
            _subscriptions: subscriptions,
        };
        this.request_tiles(cx);
        this
    }
    fn request_tiles(&mut self, cx: &mut Context<Self>) {
        let generation = self.latest.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(service) = self.service.clone() else {
            return;
        };
        let viewport = self.viewport.clone();
        let latest = self.latest.clone();
        self.loading = true;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let batch = cx
                .background_spawn(async move { service.load(&viewport, generation, &latest) })
                .await;
            this.update(cx, |this, cx| {
                if this.latest.load(Ordering::Relaxed) != generation {
                    return;
                }
                let mut tiles = Vec::new();
                for tile in batch.tiles {
                    if let Ok(bounds) = tile.id.bounds() {
                        let mut bgra = tile.pixels.clone();
                        for pixel in bgra.pixels_mut() {
                            pixel.0.swap(0, 2);
                        }
                        tiles.push(MapTile {
                            bounds,
                            pixels: tile.pixels,
                            texture: Arc::new(RenderImage::new(vec![image::Frame::new(bgra)])),
                        });
                    }
                }
                if !tiles.is_empty() {
                    let mut map = Map::new();
                    map.add_visible_layer("OpenStreetMap", ImagesVecLayer::from_images(tiles));
                    this.map = Arc::new(map);
                }
                this.loading = false;
                this.map_feedback = (batch.failures > 0)
                    .then(|| "部分地图未能加载，请检查网络后重试；也可手动填写坐标。".into());
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
    fn pan(&mut self, dx: f64, dy: f64, cx: &mut Context<Self>) {
        self.viewport.pan(dx, dy);
        self.request_tiles(cx);
    }
    fn zoom(&mut self, delta: i8, cx: &mut Context<Self>) {
        self.viewport.zoom(delta);
        self.request_tiles(cx);
    }
    fn choose(
        &mut self,
        location: Location,
        center: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(location);
        self.latitude.update(cx, |input, cx| {
            input.set_value(format!("{:.6}", location.latitude()), window, cx)
        });
        self.longitude.update(cx, |input, cx| {
            input.set_value(format!("{:.6}", location.longitude()), window, cx)
        });
        self.feedback = None;
        if center {
            if let Err(error) = self.viewport.locate(location) {
                self.feedback = Some(error.to_string().into());
            }
            self.request_tiles(cx);
        }
        cx.notify();
    }
    fn coordinates(&self, cx: &App) -> anyhow::Result<Location> {
        let lat = self
            .latitude
            .read(cx)
            .value()
            .trim()
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("请填写有效纬度"))?;
        let lon = self
            .longitude
            .read(cx)
            .value()
            .trim()
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("请填写有效经度"))?;
        Location::new(lat, lon)
    }
    fn locate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.coordinates(cx) {
            Ok(location) => self.choose(location, true, window, cx),
            Err(error) => {
                self.feedback = Some(error.to_string().into());
                cx.notify();
            }
        }
    }
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.coordinates(cx).and_then(|location| {
            self.editor_window.update(cx, |_, window, cx| {
                self.editor.update(cx, |editor, cx| {
                    window.activate_window();
                    editor.set_location(location, window, cx)
                })
            })??
        });
        match result {
            Ok(()) => {
                window.remove_window();
            }
            Err(error) => {
                self.feedback = Some(format!("无法使用位置：{error}").into());
                cx.notify();
            }
        }
    }
    fn search(&mut self, cx: &mut Context<Self>) {
        if self.searching {
            return;
        }
        let query = self.query.read(cx).value().trim().to_string();
        if query.is_empty() {
            self.feedback = Some("请输入要搜索的地点".into());
            cx.notify();
            return;
        }
        if let Some(places) = self.search_cache.get(&query) {
            self.places = places.clone();
            self.feedback = self
                .places
                .is_empty()
                .then(|| "未找到地点，请换个名称或直接在地图选点。".into());
            cx.notify();
            return;
        }
        if self
            .search_time
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            self.feedback = Some("请稍等片刻再搜索".into());
            cx.notify();
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        self.searching = true;
        self.search_time = Some(Instant::now());
        self.feedback = None;
        self.places.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let key = query.clone();
            let result = cx
                .background_spawn(async move { service.search(&query) })
                .await;
            this.update(cx, |this, cx| {
                this.searching = false;
                match result {
                    Ok(places) => {
                        this.search_cache.insert(key, places.clone());
                        this.places = places;
                        this.feedback = this
                            .places
                            .is_empty()
                            .then(|| "未找到地点，请换个名称或直接在地图选点。".into());
                    }
                    Err(_) => {
                        this.feedback =
                            Some("地点搜索失败，请检查网络；仍可在地图选点或手动填写坐标。".into())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn render_map(&self, cx: &Context<Self>) -> impl IntoElement {
        let map = self.map.clone();
        let viewport = self.viewport.clone();
        let selected = self.selected;
        let measured = self.bounds.clone();
        let owner = cx.weak_entity();
        div()
            .id("gps-map")
            .map(|this| {
                #[cfg(test)]
                let this = this.test_support();
                this
            })
            .track_focus(&self.focus)
            .tab_index(0)
            .key_context("GpsMap")
            .role(Role::Region)
            .aria_label("GPS 地图，方向键移动，加减键缩放；也可输入经纬度选点")
            .border_2()
            .border_color(cx.theme().border)
            .focus_visible(|this| this.border_color(cx.theme().ring))
            .on_action(cx.listener(|this, _: &MapLeft, _, cx| this.pan(80., 0., cx)))
            .on_action(cx.listener(|this, _: &MapRight, _, cx| this.pan(-80., 0., cx)))
            .on_action(cx.listener(|this, _: &MapUp, _, cx| this.pan(0., 80., cx)))
            .on_action(cx.listener(|this, _: &MapDown, _, cx| this.pan(0., -80., cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom(1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom(-1, cx)))
            .relative()
            .flex_1()
            .min_h(px(160.))
            .w_full()
            .overflow_hidden()
            .bg(cx.theme().muted)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.drag = Some((event.position, event.position));
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if !event.dragging() {
                    this.drag = None;
                    return;
                }
                if let Some((start, last)) = this.drag {
                    this.viewport.pan(
                        (event.position.x - last.x).as_f32() as f64,
                        (event.position.y - last.y).as_f32() as f64,
                    );
                    this.drag = Some((start, event.position));
                    this.request_tiles(cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if let Some((start, _)) = this.drag.take() {
                        let distance = (event.position.x - start.x)
                            .as_f32()
                            .hypot((event.position.y - start.y).as_f32());
                        if distance < 4.0 {
                            let p = event.position - this.bounds.get().origin;
                            if let Ok(location) = this
                                .viewport
                                .location_at(p.x.as_f32() as f64, p.y.as_f32() as f64)
                            {
                                this.choose(location, false, window, cx);
                            }
                        }
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag = None),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                let delta = match event.delta {
                    ScrollDelta::Pixels(p) => p.y.as_f32(),
                    ScrollDelta::Lines(p) => p.y,
                };
                if delta.abs() > 0.0
                    && this
                        .wheel_time
                        .is_none_or(|t| t.elapsed() > Duration::from_millis(120))
                {
                    this.wheel_time = Some(Instant::now());
                    this.zoom(if delta > 0.0 { 1 } else { -1 }, cx);
                }
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let changed = measured.get().size != bounds.size;
                        measured.set(bounds);
                        let mut viewport = viewport.clone();
                        viewport.resize(
                            bounds.size.width.as_f32() as f64,
                            bounds.size.height.as_f32() as f64,
                        );
                        if changed {
                            let owner = owner.clone();
                            let size = bounds.size;
                            cx.defer(move |cx| {
                                owner
                                    .update(cx, |this, cx| {
                                        this.viewport.resize(
                                            size.width.as_f32() as f64,
                                            size.height.as_f32() as f64,
                                        );
                                        this.request_tiles(cx);
                                    })
                                    .ok();
                            });
                        }
                        let mut renderer = TileRenderer::default();
                        if let Ok(controller) = viewport.controller() {
                            renderer.draw_map(&map, &controller, &StyleBuilder::new().into());
                        }
                        let mut elements = Vec::new();
                        for (image, rect) in renderer.0 {
                            // 相邻瓦片略重叠，避免亚像素取整留下细线。
                            let mut element = img(image)
                                .object_fit(ObjectFit::Fill)
                                .w(px(rect.width() as f32 + 0.5))
                                .h(px(rect.height() as f32 + 0.5))
                                .into_any_element();
                            element.prepaint_as_root(
                                bounds.origin
                                    + point(px(rect.min().x as f32), px(rect.min().y as f32)),
                                size(
                                    AvailableSpace::Definite(px(rect.width() as f32 + 0.5)),
                                    AvailableSpace::Definite(px(rect.height() as f32 + 0.5)),
                                ),
                                window,
                                cx,
                            );
                            elements.push(element);
                        }
                        if let Some(location) = selected
                            && let Ok((x, y)) = viewport.point(location)
                        {
                            let mut marker = div()
                                .size(px(18.))
                                .rounded_full()
                                .bg(cx.theme().primary)
                                .border_2()
                                .border_color(cx.theme().background)
                                .shadow_md()
                                .into_any_element();
                            marker.prepaint_as_root(
                                bounds.origin + point(px(x as f32 - 9.), px(y as f32 - 9.)),
                                size(
                                    AvailableSpace::Definite(px(18.)),
                                    AvailableSpace::Definite(px(18.)),
                                ),
                                window,
                                cx,
                            );
                            elements.push(marker);
                        }
                        elements
                    },
                    |_, mut elements, window, cx| {
                        for element in &mut elements {
                            element.paint(window, cx);
                        }
                    },
                )
                .size_full(),
            )
            .child(
                h_flex()
                    .absolute()
                    .top_3()
                    .right_3()
                    .gap_2()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("gps-zoom-out")
                            .label("−")
                            .outline()
                            .tooltip("缩小地图")
                            .on_click(cx.listener(|this, _, _, cx| this.zoom(-1, cx))),
                    )
                    .child(
                        Button::new("gps-zoom-in")
                            .label("+")
                            .outline()
                            .tooltip("放大地图")
                            .on_click(cx.listener(|this, _, _, cx| this.zoom(1, cx))),
                    )
                    .child(
                        Button::new("gps-map-retry")
                            .label("重试")
                            .outline()
                            .on_click(cx.listener(|this, _, _, cx| this.request_tiles(cx))),
                    ),
            )
            .child(
                Button::new("gps-attribution")
                    .absolute()
                    .bottom_2()
                    .right_2()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .label("© OpenStreetMap contributors")
                    .outline()
                    .on_click(|_, _, cx| cx.open_url("https://www.openstreetmap.org/copyright")),
            )
    }
}
impl Drop for LocationPicker {
    fn drop(&mut self) {
        self.latest.fetch_add(1, Ordering::Relaxed);
    }
}
impl Render for LocationPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 状态占用固定高度；加载结束不能改变视口大小，再触发一轮瓦片加载。
        let status = self.map_feedback.clone().map_or_else(
            || {
                description(if self.loading {
                    "正在加载地图…"
                } else {
                    " "
                })
            },
            |feedback| warning(feedback, cx),
        );
        v_flex()
            .size_full()
            .min_h_0()
            .child(TitleBar::new().child("为当前照片添加 GPS"))
            .child(
                v_flex()
                    .id("gps-picker-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&self.query).flex_1())
                            .child(
                                Button::new("gps-search")
                                    .label(if self.searching { "搜索中…" } else { "搜索" })
                                    .outline()
                                    .disabled(self.searching || self.service.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| this.search(cx))),
                            ),
                    )
                    .child(description("拖动地图移动，滚轮或 + / − 缩放，点击选点。聚焦地图后可用方向键移动。地图与搜索需要联网；搜索使用 Photon，只发送地点名称。"))
                    .when(!self.places.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .id("gps-results")
                                .max_h(px(112.))
                                .min_h_0()
                                .overflow_y_scroll()
                                .gap_2()
                                .children(self.places.iter().enumerate().map(|(ix, place)| {
                                    let location = place.location;
                                    Button::new(("gps-result", ix))
                                        .label(place.label.clone())
                                        .outline()
                                        .w_full()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.choose(location, true, window, cx)
                                        }))
                                })),
                        )
                    })
                    .child(self.render_map(cx))
                    .child(div().h_5().flex_shrink_0().child(status))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                v_flex().flex_1().gap_1()
                                    .child(description("纬度（WGS 84）"))
                                    .child(Input::new(&self.latitude)),
                            )
                            .child(
                                v_flex().flex_1().gap_1()
                                    .child(description("经度（WGS 84）"))
                                    .child(Input::new(&self.longitude)),
                            )
                            .child(
                                Button::new("gps-locate")
                                    .label("定位到坐标")
                                    .outline()
                                    .on_click(cx.listener(|this, _, window, cx| this.locate(window, cx))),
                            ),
                    )
                    .when_some(self.feedback.clone(), |this, feedback| {
                        this.child(warning(feedback, cx))
                    })
                    .child(description("使用位置后，请在 EXIF 编辑页点击“应用”。GPS 会写入导出图片，原始照片不变。"))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("gps-cancel")
                                    .label("取消")
                                    .outline()
                                    .on_click(|_, window, _| window.remove_window()),
                            )
                            .child(
                                Button::new("gps-confirm")
                                    .label("使用此位置")
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartography_raster_placement_matches_pointer_coordinates() {
        let id = crate::features::geolocation::TileId {
            zoom: 14,
            x: 13380,
            y: 7137,
        };
        let bounds = id.bounds().unwrap();
        let center = Location::new(bounds.center().y, bounds.center().x).unwrap();
        let viewport = MapViewport::new(Some(center));
        let pixels = image::RgbaImage::new(256, 256);
        let texture = Arc::new(RenderImage::new(vec![image::Frame::new(pixels.clone())]));
        let mut map = Map::new();
        map.add_visible_layer(
            "test",
            ImagesVecLayer::from_images(vec![MapTile {
                bounds,
                pixels,
                texture,
            }]),
        );
        let mut renderer = TileRenderer::default();
        renderer.draw_map(
            &map,
            &viewport.controller().unwrap(),
            &StyleBuilder::new().into(),
        );
        assert_eq!(renderer.0.len(), 1);
        let rect = renderer.0[0].1;
        let corner = viewport
            .point(Location::new(bounds.max().y, bounds.min().x).unwrap())
            .unwrap();
        assert!((rect.min().x - corner.0).abs() < 1e-7);
        assert!((rect.min().y - corner.1).abs() < 1e-7);
        assert!((rect.width() - 256.).abs() < 1e-6);
        assert!((rect.height() - 256.).abs() < 1e-6);
    }
}
