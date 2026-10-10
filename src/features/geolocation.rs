//! 照片定位：WGS 84 坐标、cartography 视口和带本地缓存的地图请求。

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use cartography::{
    Projection, ViewController,
    geo::{Coord, Rect},
};
use nom_exif::GPSInfo;
use serde::{Deserialize, Serialize};

mod map_source;
pub(crate) use map_source::MapCoordinates;
pub use map_source::{MapProvider, MapSettings};

const HALF_WORLD: f64 = 20_037_508.342_789_244;
const CACHE_AGE: u64 = 7 * 24 * 60 * 60;
// 下载写入与清理共用进程级锁，防止清理期间被另一个地图窗口写回瓦片。
static TILE_CACHE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn tile_cache_directory() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .context("无法找到地图缓存目录")?
        .join("lumen-frame/maps"))
}

pub(crate) fn clear_tile_cache() -> Result<()> {
    clear_tile_cache_at(&tile_cache_directory()?)
}

fn clear_tile_cache_at(directory: &std::path::Path) -> Result<()> {
    let _guard = TILE_CACHE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("地图缓存状态异常，请重启应用后重试"))?;
    match std::fs::remove_dir_all(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("清理地图瓦片缓存失败，请检查缓存目录权限"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    latitude: f64,
    longitude: f64,
}

impl Location {
    pub fn new(latitude: f64, longitude: f64) -> Result<Self> {
        if !latitude.is_finite()
            || !(-90.0..=90.0).contains(&latitude)
            || !longitude.is_finite()
            || !(-180.0..=180.0).contains(&longitude)
        {
            bail!("纬度应在 -90 到 90 之间，经度应在 -180 到 180 之间");
        }
        Ok(Self {
            latitude,
            longitude,
        })
    }

    pub fn from_gps(gps: &GPSInfo) -> Option<Self> {
        Self::new(gps.latitude_decimal()?, gps.longitude_decimal()?).ok()
    }
    pub fn latitude(self) -> f64 {
        self.latitude
    }
    pub fn longitude(self) -> f64 {
        self.longitude
    }
    pub fn to_gps(self) -> Result<GPSInfo> {
        format!("{:+010.6}{:+011.6}/", self.latitude, self.longitude)
            .parse()
            .context("无法转换 GPS 坐标")
    }
    fn projected(self) -> Result<Coord> {
        cartography::transform(
            &Projection::wgs84(),
            &Projection::web_mercator(),
            &Coord {
                x: self.longitude,
                y: self.latitude.clamp(-85.051_128_78, 85.051_128_78),
            },
        )
    }
}

/// 仅保存地图交互状态，照片 GPS 的真值由 AppView 应用到队列。
#[derive(Clone)]
pub(crate) struct MapViewport {
    center: Coord,
    coordinates: MapCoordinates,
    zoom: u8,
    width: f64,
    height: f64,
}

impl MapViewport {
    pub fn new(location: Option<Location>) -> Self {
        let center = location
            .unwrap_or(Location {
                latitude: 35.0,
                longitude: 105.0,
            })
            .projected()
            .expect("有效地图中心");
        Self {
            center,
            coordinates: MapCoordinates::Wgs84,
            zoom: if location.is_some() { 14 } else { 3 },
            width: 800.0,
            height: 380.0,
        }
    }
    pub fn set_coordinates(&mut self, coordinates: MapCoordinates) -> Result<()> {
        let p = cartography::transform(
            &Projection::web_mercator(),
            &Projection::wgs84(),
            &self.center,
        )?;
        let location = self.coordinates.gps_location(p.y, p.x)?;
        self.center = coordinates.map_location(location)?.projected()?;
        self.coordinates = coordinates;
        self.clamp_center();
        Ok(())
    }
    fn resolution(&self) -> f64 {
        2.0 * HALF_WORLD / (256.0 * 2.0f64.powi(self.zoom as i32))
    }
    pub fn resize(&mut self, width: f64, height: f64) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        self.zoom = self.zoom.max(self.minimum_zoom());
        self.clamp_center();
    }
    fn minimum_zoom(&self) -> u8 {
        ((self.width.max(self.height) / 256.0).log2().ceil() as u8).clamp(3, 19)
    }
    fn clamp_center(&mut self) {
        let x = (HALF_WORLD - self.width * self.resolution() / 2.0).max(0.0);
        let y = (HALF_WORLD - self.height * self.resolution() / 2.0).max(0.0);
        self.center.x = self.center.x.clamp(-x, x);
        self.center.y = self.center.y.clamp(-y, y);
    }
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.center.x -= dx * self.resolution();
        self.center.y += dy * self.resolution();
        self.clamp_center();
    }
    pub fn zoom(&mut self, delta: i8) {
        self.zoom = (self.zoom as i8 + delta).clamp(self.minimum_zoom() as i8, 19) as u8;
        self.clamp_center();
    }
    pub fn locate(&mut self, location: Location) -> Result<()> {
        self.center = self.coordinates.map_location(location)?.projected()?;
        self.zoom = 15.max(self.minimum_zoom());
        self.clamp_center();
        Ok(())
    }
    pub fn rect(&self) -> Rect {
        let (w, h) = (
            self.width * self.resolution() / 2.0,
            self.height * self.resolution() / 2.0,
        );
        Rect::new(
            (self.center.x - w, self.center.y - h),
            (self.center.x + w, self.center.y + h),
        )
    }
    pub fn controller(&self) -> Result<ViewController> {
        let rect = cartography::transform(
            &Projection::web_mercator(),
            &Projection::wgs84(),
            &self.rect(),
        )?;
        let mut controller =
            ViewController::show_extent(rect, Rect::new((0.0, 0.0), (self.width, self.height)));
        controller.set_projection(Projection::web_mercator())?;
        Ok(controller)
    }
    pub fn location_at(&self, x: f64, y: f64) -> Result<Location> {
        let p = Coord {
            x: self.rect().min().x + x * self.resolution(),
            y: self.rect().max().y - y * self.resolution(),
        };
        let p = cartography::transform(&Projection::web_mercator(), &Projection::wgs84(), &p)?;
        self.coordinates.gps_location(p.y, p.x)
    }
    pub fn point(&self, location: Location) -> Result<(f64, f64)> {
        let p = self.coordinates.map_location(location)?.projected()?;
        Ok((
            (p.x - self.rect().min().x) / self.resolution(),
            (self.rect().max().y - p.y) / self.resolution(),
        ))
    }
    fn tiles(&self) -> Vec<TileId> {
        let n = 1u32 << self.zoom;
        let tile_span = 2.0 * HALF_WORLD / n as f64;
        let rect = self.rect();
        let index = |value: f64| (value / tile_span).floor().clamp(0.0, n as f64 - 1.0) as u32;
        let (x0, x1) = (
            index(rect.min().x + HALF_WORLD),
            index(rect.max().x + HALF_WORLD),
        );
        let (y0, y1) = (
            index(HALF_WORLD - rect.max().y),
            index(HALF_WORLD - rect.min().y),
        );
        (x0..=x1)
            .flat_map(|x| {
                (y0..=y1).map(move |y| TileId {
                    zoom: self.zoom,
                    x,
                    y,
                })
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TileId {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
}
impl TileId {
    pub fn bounds(self) -> Result<Rect> {
        let span = 2.0 * HALF_WORLD / (1u32 << self.zoom) as f64;
        let rect = Rect::new(
            (
                self.x as f64 * span - HALF_WORLD,
                HALF_WORLD - (self.y + 1) as f64 * span,
            ),
            (
                (self.x + 1) as f64 * span - HALF_WORLD,
                HALF_WORLD - self.y as f64 * span,
            ),
        );
        cartography::transform(&Projection::web_mercator(), &Projection::wgs84(), &rect)
    }
}

pub(crate) struct LoadedTile {
    pub id: TileId,
    pub pixels: image::RgbaImage,
}
pub(crate) struct TileBatch {
    pub tiles: Vec<LoadedTile>,
    pub failures: usize,
}

#[derive(Clone)]
pub(crate) struct MapService {
    client: reqwest::blocking::Client,
    cache: PathBuf,
    settings: MapSettings,
    search_url: String,
}

#[derive(Default, Serialize, Deserialize)]
struct TileMetadata {
    expires: u64,
    etag: Option<String>,
    modified: Option<String>,
}

impl MapService {
    pub fn from_settings(settings: &MapSettings) -> Result<Self> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::hash::DefaultHasher::new();
        settings.tile_template().hash(&mut hash);
        let cache = tile_cache_directory()?.join(format!("{:x}", hash.finish()));
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!(
                "LumenFrame/",
                env!("CARGO_PKG_VERSION"),
                " (contact: cakeal@qq.com)"
            ))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            client,
            cache,
            settings: settings.clone(),
            search_url: std::env::var("LUMEN_FRAME_GEOCODER_URL")
                .unwrap_or_else(|_| "https://photon.komoot.io/api/".into()),
        })
    }
    pub fn load(
        &self,
        viewport: &MapViewport,
        generation: u64,
        latest: &Arc<AtomicU64>,
    ) -> TileBatch {
        let mut result = TileBatch {
            tiles: Vec::new(),
            failures: 0,
        };
        let Ok(_guard) = TILE_CACHE_LOCK.lock() else {
            return result;
        };
        for id in viewport.tiles() {
            if latest.load(Ordering::Relaxed) != generation {
                break;
            }
            match self.tile(id) {
                Ok(pixels) => result.tiles.push(LoadedTile { id, pixels }),
                Err(_) => {
                    result.failures += 1;
                    if result.failures >= 3 {
                        break;
                    }
                }
            }
        }
        result
    }
    fn tile(&self, id: TileId) -> Result<image::RgbaImage> {
        let path = self
            .cache
            .join(format!("{}/{}/{}.png", id.zoom, id.x, id.y));
        let meta_path = path.with_extension("json");
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let meta: TileMetadata = std::fs::read(&meta_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let cached = std::fs::read(&path)
            .ok()
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|image| image.to_rgba8());
        if meta.expires > now
            && let Some(image) = &cached
        {
            return Ok(image.clone());
        }
        let url = self.settings.tile_url(id);
        let mut request = self.client.get(url);
        if cached.is_some() {
            if let Some(tag) = &meta.etag {
                request = request.header("If-None-Match", tag);
            }
            if let Some(modified) = &meta.modified {
                request = request.header("If-Modified-Since", modified);
            }
        }
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => return cached.context(error),
        };
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let max_age = header("cache-control")
            .and_then(|value| {
                value.split(',').find_map(|part| {
                    part.trim()
                        .strip_prefix("max-age=")
                        .and_then(|seconds| seconds.parse::<u64>().ok())
                })
            })
            .unwrap_or(CACHE_AGE);
        let next = TileMetadata {
            expires: now + max_age,
            etag: header("etag").or(meta.etag),
            modified: header("last-modified").or(meta.modified),
        };
        let (pixels, bytes) = if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            (cached.context("地图缓存已失效")?, None)
        } else {
            if !response.status().is_success() {
                return cached.context("地图服务暂时不可用");
            }
            let bytes = response.bytes()?.to_vec();
            let pixels = image::load_from_memory(&bytes)
                .context("地图瓦片格式无效")?
                .to_rgba8();
            (pixels, Some(bytes))
        };
        // 缓存不可写时仍可使用本次下载的地图，不能让后台线程 panic。
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_ok()
        {
            if let Some(bytes) = bytes {
                let temporary = path.with_extension("tmp");
                if std::fs::write(&temporary, bytes).is_ok() {
                    let _ = std::fs::rename(temporary, &path);
                }
            }
            if let Ok(bytes) = serde_json::to_vec(&next) {
                let _ = std::fs::write(meta_path, bytes);
            }
        }
        Ok(pixels)
    }
    pub fn search(&self, query: &str) -> Result<Vec<Place>> {
        let value: serde_json::Value = self
            .client
            .get(&self.search_url)
            .query(&[("q", query), ("limit", "5")])
            .send()?
            .error_for_status()?
            .json()?;
        Ok(value["features"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|feature| {
                let coordinates = feature["geometry"]["coordinates"].as_array()?;
                let location = Location::new(
                    coordinates.get(1)?.as_f64()?,
                    coordinates.first()?.as_f64()?,
                )
                .ok()?;
                let parts: Vec<&str> = ["name", "city", "state", "country"]
                    .iter()
                    .filter_map(|key| feature["properties"][key].as_str())
                    .collect();
                Some(Place {
                    location,
                    label: parts.join(" · "),
                })
            })
            .collect())
    }
}

#[derive(Clone)]
pub(crate) struct Place {
    pub location: Location,
    pub label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_tiles_removes_all_sources_but_preserves_other_app_cache() {
        let root = std::env::temp_dir().join(format!(
            "lumen-frame-tile-cache-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let maps = root.join("maps");
        for source in ["osm", "tencent"] {
            let directory = maps.join(source).join("14/13386");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("7151.png"), b"tile").unwrap();
            std::fs::write(directory.join("7151.json"), b"metadata").unwrap();
        }
        let unrelated = root.join("update-check.stamp");
        std::fs::write(&unrelated, b"keep").unwrap();
        clear_tile_cache_at(&maps).unwrap();
        assert!(!maps.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
        clear_tile_cache_at(&maps).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validates_and_roundtrips_gps_coordinates() {
        for (lat, lon) in [
            (22.5429, 114.0596),
            (-33.8688, 151.2093),
            (40.7128, -74.006),
            (90., 180.),
            (-90., -180.),
        ] {
            let location = Location::new(lat, lon).unwrap();
            let parsed = Location::from_gps(&location.to_gps().unwrap()).unwrap();
            assert!((parsed.latitude - lat).abs() < 0.00001);
            assert!((parsed.longitude - lon).abs() < 0.00001);
        }
        assert!(Location::new(91., 0.).is_err());
        assert!(Location::new(0., 181.).is_err());
        assert!(Location::new(f64::NAN, 0.).is_err());
    }

    #[test]
    fn map_projection_matches_clicks_after_pan_zoom_and_resize() {
        let location = Location::new(22.5429, 114.0596).unwrap();
        let mut viewport = MapViewport::new(Some(location));
        viewport.resize(920., 440.);
        viewport.pan(72., -31.);
        viewport.zoom(2);
        let (x, y) = viewport.point(location).unwrap();
        let clicked = viewport.location_at(x, y).unwrap();
        assert!((clicked.latitude - location.latitude).abs() < 1e-8);
        assert!((clicked.longitude - location.longitude).abs() < 1e-8);
        let controller = viewport.controller().unwrap();
        assert!((controller.map_rect().width() - viewport.rect().width()).abs() < 1e-6);
        assert!(viewport.tiles().len() <= 30);
        viewport.locate(Location::new(90., 180.).unwrap()).unwrap();
        viewport.resize(920., 440.);
        assert!(viewport.controller().is_ok());
        assert!(viewport.location_at(0., 0.).is_ok());
    }

    #[test]
    fn tencent_coordinates_preserve_gps_when_switching_sources() {
        for (lat, lon) in [(39.915, 116.404), (22.5429, 114.0596), (35.6762, 139.6503)] {
            let location = Location::new(lat, lon).unwrap();
            let mut viewport = MapViewport::new(Some(location));
            viewport.resize(800., 380.);
            viewport.set_coordinates(MapCoordinates::Gcj02).unwrap();
            let center = viewport.location_at(400., 190.).unwrap();
            assert!((center.latitude() - lat).abs() < 1e-8);
            assert!((center.longitude() - lon).abs() < 1e-8);
            viewport.pan(50., -30.);
            viewport.zoom(2);
            let (x, y) = viewport.point(location).unwrap();
            let selected = viewport.location_at(x, y).unwrap();
            let gps = Location::from_gps(&selected.to_gps().unwrap()).unwrap();
            assert!((gps.latitude() - lat).abs() < 1e-6);
            assert!((gps.longitude() - lon).abs() < 1e-6);
            let panned = viewport.location_at(400., 190.).unwrap();
            viewport.set_coordinates(MapCoordinates::Wgs84).unwrap();
            let restored = viewport.location_at(400., 190.).unwrap();
            assert!((restored.latitude() - panned.latitude()).abs() < 1e-8);
            assert!((restored.longitude() - panned.longitude()).abs() < 1e-8);
        }
        // 独立参考坐标：北京 WGS 84 在腾讯底图上需要偏移数百米。
        let display = MapCoordinates::Gcj02
            .map_location(Location::new(39.915, 116.404).unwrap())
            .unwrap();
        assert!((display.latitude() - 39.91640428150164).abs() < 1e-8);
        assert!((display.longitude() - 116.41024449916938).abs() < 1e-8);
    }

    #[test]
    fn builtin_sources_use_the_correct_tile_row_without_a_key() {
        let id = TileId {
            zoom: 3,
            x: 6,
            y: 3,
        };
        let tencent = MapSettings::default().with_provider(MapProvider::Tencent);
        assert_eq!(
            tencent.tile_url(id),
            "https://rt0.map.gtimg.com/tile?z=3&x=6&y=4&type=vector&styleid=1"
        );
        assert_eq!(
            MapSettings::default().tile_url(id),
            "https://tile.openstreetmap.org/3/6/3.png"
        );
    }
}
