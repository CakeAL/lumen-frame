//! 地图源偏好及瓦片地址；与照片的 WGS 84 GPS 配置分开保存。

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{Location, TileId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapProvider {
    Tencent,
    #[default]
    #[serde(other)]
    OpenStreetMap,
}

impl MapProvider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenStreetMap => "OpenStreetMap",
            Self::Tencent => "腾讯地图（公开瓦片）",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum MapCoordinates {
    Wgs84,
    Gcj02,
}

impl MapCoordinates {
    pub(crate) fn map_location(self, location: Location) -> Result<Location> {
        let (lon, lat) = match self {
            Self::Wgs84 => (location.longitude(), location.latitude()),
            Self::Gcj02 => {
                coordtransform::wgs84_to_gcj02(location.longitude(), location.latitude())
            }
        };
        Location::new(lat, lon)
    }

    pub(crate) fn gps_location(self, latitude: f64, longitude: f64) -> Result<Location> {
        let (lon, lat) = match self {
            Self::Wgs84 => (longitude, latitude),
            Self::Gcj02 => coordtransform::gcj02_to_wgs84(longitude, latitude),
        };
        Location::new(lat, lon)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapSettings {
    provider: MapProvider,
}

impl MapSettings {
    pub fn provider(&self) -> MapProvider {
        self.provider
    }
    pub fn with_provider(mut self, provider: MapProvider) -> Self {
        self.provider = provider;
        self
    }
    pub(crate) fn coordinates(&self) -> MapCoordinates {
        match self.provider {
            MapProvider::OpenStreetMap => MapCoordinates::Wgs84,
            MapProvider::Tencent => MapCoordinates::Gcj02,
        }
    }
    pub(crate) fn tile_template(&self) -> &str {
        match self.provider {
            MapProvider::OpenStreetMap => "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
            // 腾讯瓦片使用自下向上的 TMS 行号，绘制仍使用标准 XYZ 边界。
            MapProvider::Tencent => {
                "https://rt0.map.gtimg.com/tile?z={z}&x={x}&y={-y}&type=vector&styleid=1"
            }
        }
    }
    pub(crate) fn attribution(&self) -> (&str, &str) {
        match self.provider {
            MapProvider::OpenStreetMap => (
                "© OpenStreetMap contributors",
                "https://www.openstreetmap.org/copyright",
            ),
            MapProvider::Tencent => ("© 腾讯地图", "https://map.qq.com/"),
        }
    }
    pub(crate) fn tile_url(&self, id: TileId) -> String {
        self.tile_template()
            .replace("{z}", &id.zoom.to_string())
            .replace("{x}", &id.x.to_string())
            .replace("{y}", &id.y.to_string())
            .replace("{-y}", &((1u32 << id.zoom) - 1 - id.y).to_string())
    }
}
