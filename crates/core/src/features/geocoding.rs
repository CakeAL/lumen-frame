//! 逆地理编码的本机配置、字段路径及按坐标复用的高德响应。
use super::geolocation::{Location, MapCoordinates};
use crate::{media::ExifInfo, watermark::TextGroup};
use anyhow::{Context as _, Result, bail, ensure};
use reverse_geocoder_cn::{Address, ReverseGeocoder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    io::Read,
    sync::{LazyLock, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const DOCUMENTATION_URL: &str = "https://lbs.amap.com/api/webservice/guide/api/georegeo#s6";
const ENDPOINT: &str = "https://restapi.amap.com/v3/geocode/regeo";
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeocodingProvider {
    #[default]
    Builtin,
    Amap,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressField {
    pub id: u64,
    pub paths: String,
}
impl AddressField {
    pub fn token(&self) -> Result<String> {
        Ok(format!(
            "{{{}}}",
            paths(&self.paths)?
                .iter()
                .map(|p| path_label(p))
                .collect::<Vec<_>>()
                .join("|")
        ))
    }
    pub fn render(&self, response: &Value) -> Result<String> {
        Ok(paths(&self.paths)?
            .iter()
            .map(|path| value_at(response, path))
            .collect::<String>())
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeocodingSettings {
    pub provider: GeocodingProvider,
    pub api_key: String,
    pub fields: Vec<AddressField>,
}
// 配置会进入导出快照的 Debug 信息，不能把本机密钥带到日志中。
impl fmt::Debug for GeocodingSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeocodingSettings")
            .field("provider", &self.provider)
            .field("api_key", &"[已隐藏]")
            .field("fields", &self.fields)
            .finish()
    }
}
impl Default for GeocodingSettings {
    fn default() -> Self {
        Self { provider: GeocodingProvider::Builtin, api_key: String::new(), fields: [
            "addressComponent.province,addressComponent.city,addressComponent.district,addressComponent.township",
            "formatted_address", "pois.0.name",
        ].into_iter().enumerate().map(|(ix, paths)| AddressField { id: ix as u64 + 1, paths: paths.into() }).collect() }
    }
}
impl GeocodingSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.provider != GeocodingProvider::Amap || !self.api_key.trim().is_empty(),
            "请输入高德 Web 服务 API Key，再保存解析设置"
        );
        ensure!(self.api_key.len() <= 512, "高德 API Key 过长，请检查输入");
        ensure!(self.fields.len() <= 64, "最多可配置 64 组地址字段");
        let (mut ids, mut tokens) = (HashSet::new(), HashSet::new());
        for field in &self.fields {
            ensure!(field.id < u64::MAX, "地址字段编号无效，请重新添加该字段");
            ensure!(ids.insert(field.id), "地址字段编号重复，请重新添加该字段");
            ensure!(
                tokens.insert(field.token()?),
                "地址字段生成了重复模板，请修改字段路径"
            );
        }
        Ok(())
    }
    pub fn tokens(&self) -> Vec<String> {
        self.fields.iter().filter_map(|f| f.token().ok()).collect()
    }
    fn needed(&self, groups: &[TextGroup]) -> bool {
        groups
            .iter()
            .flat_map(|g| &g.text.template)
            .any(|t| self.uses_fields(t))
    }
    pub fn uses_fields(&self, template: &str) -> bool {
        ["{省}", "{市}", "{区}"]
            .into_iter()
            .any(|v| template.contains(v))
            || self.tokens().iter().any(|v| template.contains(v))
    }
}

fn paths(input: &str) -> Result<Vec<&str>> {
    ensure!(
        !input.trim().is_empty() && input.len() <= 4096,
        "请填写地址字段路径，最长 4096 字符"
    );
    let result: Vec<_> = input
        .split(',')
        .map(|p| p.trim().strip_prefix("regeocode.").unwrap_or(p.trim()))
        .collect();
    ensure!(result.len() <= 32, "每组最多拼接 32 个地址字段");
    for path in &result {
        ensure!(
            !path.is_empty()
                && path
                    .split('.')
                    .all(|s| !s.is_empty()
                        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')),
            "字段路径格式无效：使用点分隔层级、数字选择数组项、英文逗号拼接多个字段"
        );
    }
    Ok(result)
}
fn path_label(path: &str) -> String {
    if let Some(leaf) = path.strip_prefix("addressComponent.") {
        return match leaf {
            "province" => "省",
            "city" => "市",
            "district" => "区",
            "township" => "街道",
            "country" => "国家",
            "citycode" => "城市编码",
            "adcode" => "行政区编码",
            "towncode" => "乡镇街道编码",
            "seaArea" => "所属海域",
            "neighborhood.name" => "社区名称",
            "neighborhood.type" => "社区类型",
            "building.name" => "建筑名称",
            "building.type" => "建筑类型",
            "streetNumber.street" => "道路名称",
            "streetNumber.number" => "门牌号",
            "streetNumber.location" => "门牌坐标",
            "streetNumber.direction" => "门牌方向",
            "streetNumber.distance" => "门牌距离",
            _ => return generic_label(leaf),
        }
        .into();
    }
    if path == "formatted_address" {
        return "格式化地址".into();
    }
    generic_label(path)
}
fn generic_label(path: &str) -> String {
    path.split('.')
        .map(|p| match p {
            "pois" => "兴趣点",
            "aois" => "区域",
            "roads" => "道路",
            "roadinters" => "道路交叉口",
            "businessAreas" => "商圈",
            "name" => "名称",
            "id" => "编号",
            "type" => "类型",
            "tel" => "电话",
            "distance" => "距离",
            "direction" => "方向",
            "address" => "地址",
            "location" => "坐标",
            "businessarea" => "商圈",
            "adcode" => "行政区编码",
            "area" => "面积",
            "first_id" => "道路一编号",
            "first_name" => "道路一名称",
            "second_id" => "道路二编号",
            "second_name" => "道路二名称",
            _ => p,
        })
        .collect()
}
fn value_at(root: &Value, path: &str) -> String {
    let mut current = root;
    for segment in path.split('.') {
        current = match current {
            Value::Array(values) => segment.parse::<usize>().ok().and_then(|ix| values.get(ix)),
            Value::Object(values) => values.get(segment),
            _ => None,
        }
        .unwrap_or(&Value::Null);
    }
    match current {
        Value::Null => String::new(),
        Value::String(v) => v.clone(),
        Value::Array(v) if v.is_empty() => String::new(),
        Value::Object(v) if v.is_empty() => String::new(),
        other => other.to_string(),
    }
}
fn field_values(
    settings: &GeocodingSettings,
    response: &Value,
) -> Result<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    for (name, path) in [
        ("省", "addressComponent.province"),
        ("市", "addressComponent.city"),
        ("区", "addressComponent.district"),
    ] {
        fields.insert(name.into(), value_at(response, path));
    }
    for field in &settings.fields {
        let token = field.token()?;
        fields.insert(token[1..token.len() - 1].into(), field.render(response)?);
    }
    Ok(fields)
}

pub(crate) fn administrative_address(gps: Option<&nom_exif::GPSInfo>) -> Option<Address> {
    type AddressCache = HashMap<(u64, u64), Option<Address>>;
    static GEOCODER: OnceLock<Option<ReverseGeocoder>> = OnceLock::new();
    static CACHE: LazyLock<Mutex<AddressCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    let location = Location::from_gps(gps?)?;
    let key = (
        location.latitude().to_bits(),
        location.longitude().to_bits(),
    );
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(address) = cache.get(&key) {
        return address.clone();
    }
    let address = GEOCODER
        .get_or_init(|| ReverseGeocoder::embedded().ok())
        .as_ref()
        .and_then(|g| g.reverse_geocode(location.latitude(), location.longitude()));
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(key, address.clone());
    address
}
fn builtin_response(exif: &ExifInfo) -> Value {
    administrative_address(exif.gps_info.as_ref()).map(|a| json!({"addressComponent":{"province":a.province.name,"city":a.city.name,"district":a.county.name}})).unwrap_or_else(|| json!({}))
}

#[derive(Clone)]
struct CachedResponse {
    at: Instant,
    value: std::result::Result<Value, String>,
}
type RequestKey = (String, String);
static RESPONSES: LazyLock<Mutex<HashMap<RequestKey, CachedResponse>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn request_key(exif: &ExifInfo, settings: &GeocodingSettings) -> Option<RequestKey> {
    let gps = Location::from_gps(exif.gps_info.as_ref()?)?;
    let point = MapCoordinates::Gcj02.map_location(gps).ok()?;
    Some((
        settings.api_key.trim().into(),
        format!("{:.6},{:.6}", point.longitude(), point.latitude()),
    ))
}
fn fresh(entry: &CachedResponse) -> bool {
    entry.at.elapsed()
        < if entry.value.is_ok() {
            Duration::from_secs(3600)
        } else {
            Duration::from_secs(15)
        }
}
fn request(key: &RequestKey) -> Result<Value> {
    request_at(key, ENDPOINT)
}
fn request_at(key: &RequestKey, endpoint: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法初始化高德连接")?;
    let response = client
        .get(endpoint)
        .query(&[
            ("key", key.0.as_str()),
            ("location", &key.1),
            ("extensions", "all"),
            ("output", "JSON"),
        ])
        .send()
        .map_err(|_| anyhow::anyhow!("连接高德失败，请检查网络后重试"))?;
    ensure!(
        response.status().is_success(),
        "高德服务暂不可用，请稍后重试"
    );
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("读取高德响应失败，请重试"))?;
    ensure!(bytes.len() <= 1024 * 1024, "高德响应过大，请稍后重试");
    decode_response(&bytes)
}
fn decode_response(bytes: &[u8]) -> Result<Value> {
    let value: Value = serde_json::from_slice(bytes).context("高德响应格式无效，请稍后重试")?;
    if value["status"] != "1" {
        let message = match value["infocode"].as_str().unwrap_or_default() {
            "10001" => "高德 Key 无效，请检查 Web 服务 API Key",
            "10003" | "10004" | "10044" => "高德调用量已超限，请检查配额或稍后重试",
            _ => "高德拒绝解析请求，请检查 Web 服务 Key、访问权限和调用配额",
        };
        bail!("{message}");
    }
    ensure!(
        value["regeocode"].is_object(),
        "高德未返回地址信息，请检查照片 GPS"
    );
    Ok(value["regeocode"].clone())
}
/// 仅从后台合成入口调用；无相关模板或无 GPS 时不访问网络。
pub fn resolve_exif(
    exif: Option<&ExifInfo>,
    settings: &GeocodingSettings,
    groups: &[TextGroup],
) -> Result<Option<ExifInfo>> {
    let Some(exif) = exif else { return Ok(None) };
    if !settings.needed(groups) {
        return Ok(Some(exif.clone()));
    }
    let response = if settings.provider == GeocodingProvider::Builtin {
        builtin_response(exif)
    } else if let Some(key) = request_key(exif, settings) {
        ensure!(!key.0.is_empty(), "请在设置中配置高德 Web 服务 API Key");
        // 合并并发的相同请求；UI 的缓存读取使用 try_lock，不会在主线程等待网络。
        let mut cache = RESPONSES
            .lock()
            .map_err(|_| anyhow::anyhow!("地址缓存异常，请重启应用"))?;
        let entry = if let Some(entry) = cache.get(&key).filter(|e| fresh(e)) {
            entry.clone()
        } else {
            let entry = CachedResponse {
                at: Instant::now(),
                value: request(&key).map_err(|e| e.to_string()),
            };
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(key, entry.clone());
            entry
        };
        entry.value.map_err(anyhow::Error::msg)?
    } else {
        json!({})
    };
    let mut resolved = exif.clone();
    resolved.address_fields = Some(field_values(settings, &response)?);
    Ok(Some(resolved))
}
/// 编辑器只读取已完成的地址结果，不在渲染阶段发起请求。
pub fn cached_exif(exif: &ExifInfo, settings: &GeocodingSettings) -> ExifInfo {
    let response = if settings.provider == GeocodingProvider::Builtin {
        builtin_response(exif)
    } else {
        request_key(exif, settings)
            .and_then(|key| {
                RESPONSES.try_lock().ok().and_then(|c| {
                    c.get(&key)
                        .filter(|e| fresh(e))
                        .and_then(|e| e.value.clone().ok())
                })
            })
            .unwrap_or_else(|| json!({}))
    };
    let mut resolved = exif.clone();
    resolved.address_fields = field_values(settings, &response).ok();
    resolved
}
/// 设置页的静态示例；不含用户坐标，也不调用 API。
pub fn example_response() -> Value {
    json!({"addressComponent":{"province":"北京市","city":[],"district":"海淀区","township":"燕园街道"},"formatted_address":"北京市海淀区颐和园路5号北京大学","pois":[{"name":"北京大学"}]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::text::{render_styled_template_preview, render_watermark_template};
    use crate::watermark::{TextParams, TextSpan, TextStyle};
    use std::io::Write;
    fn exif() -> ExifInfo {
        ExifInfo {
            gps_info: Some(Location::new(39.991, 116.31).unwrap().to_gps().unwrap()),
            ..Default::default()
        }
    }
    fn group(template: &str) -> Vec<TextGroup> {
        let mut group = TextGroup::default();
        group.text.template = vec![template.into()];
        vec![group]
    }
    #[test]
    fn examples_form_the_requested_tokens_and_concatenate_values() {
        let settings = GeocodingSettings::default();
        assert_eq!(
            settings.tokens(),
            vec!["{省|市|区|街道}", "{格式化地址}", "{兴趣点0名称}"]
        );
        let values = field_values(&settings, &example_response()).unwrap();
        assert_eq!(values["省|市|区|街道"], "北京市海淀区燕园街道");
        assert_eq!(values["格式化地址"], "北京市海淀区颐和园路5号北京大学");
        assert_eq!(values["兴趣点0名称"], "北京大学");
        let mut exif = exif();
        exif.address_fields = Some(values);
        let token = "{省|市|区|街道}";
        assert_eq!(
            render_watermark_template(token, &exif, "", ""),
            "北京市海淀区燕园街道"
        );
        let style = TextStyle {
            bold: Some(true),
            ..Default::default()
        };
        let params = TextParams {
            spans: vec![TextSpan {
                start: 0,
                end: token.len(),
                style: style.clone(),
            }],
            ..Default::default()
        };
        let (text, spans) = render_styled_template_preview(token, &params, &exif, "", "");
        assert_eq!(text, "北京市海淀区燕园街道");
        assert!(spans.iter().all(|span| span.style == style));
        assert!(!spans.is_empty());
    }
    #[test]
    fn arbitrary_paths_arrays_missing_values_and_validation() {
        let response = json!({"pois":[{"name":"一号"},{"name":"二号","distance":0}],"empty":[],"null":null,"flag":true});
        let field = AddressField {
            id: 1,
            paths: " regeocode.pois.1.name,pois.1.distance,pois.4.name,empty,null,flag ".into(),
        };
        assert_eq!(field.render(&response).unwrap(), "二号0true");
        assert_eq!(
            AddressField {
                id: 1,
                paths: "addressComponent.streetNumber.number".into()
            }
            .token()
            .unwrap(),
            "{门牌号}"
        );
        for invalid in [
            "",
            "pois..name",
            "pois[0].name",
            "pois.0.name,",
            "bad{token}",
        ] {
            assert!(
                AddressField {
                    id: 1,
                    paths: invalid.into()
                }
                .token()
                .is_err()
            );
        }
        let mut settings = GeocodingSettings::default();
        settings.fields.push(settings.fields[0].clone());
        assert!(settings.validate().is_err());
        assert!(
            decode_response(br#"{"status":"0","infocode":"10001"}"#)
                .unwrap_err()
                .to_string()
                .contains("Key")
        );
        assert!(decode_response(br#"{"status":"1","regeocode":[]}"#).is_err());
    }
    #[test]
    fn builtin_and_no_gps_do_not_require_a_key_or_network() {
        let settings = GeocodingSettings::default();
        let resolved = resolve_exif(
            Some(&exif()),
            &settings,
            &group("{省}{市}{区}{省|市|区|街道}"),
        )
        .unwrap()
        .unwrap();
        assert!(!resolved.address_fields.unwrap()["省"].is_empty());
        let amap = GeocodingSettings {
            provider: GeocodingProvider::Amap,
            ..settings
        };
        assert!(resolve_exif(Some(&exif()), &amap, &group("{型号}")).is_ok());
        let empty = resolve_exif(Some(&ExifInfo::default()), &amap, &group("{格式化地址}"))
            .unwrap()
            .unwrap();
        assert_eq!(
            render_watermark_template("{格式化地址}", &empty, "", ""),
            ""
        );
        assert!(resolve_exif(Some(&exif()), &amap, &group("{省}")).is_err());
    }
    #[test]
    fn request_uses_all_fields_and_converted_longitude_latitude_without_exposing_key() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/regeo", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0u8; 512];
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains("extensions=all"));
            assert!(request.contains("output=JSON"));
            assert!(request.contains("location=116."));
            assert!(request.contains("%2C39."));
            let body = json!({"status":"1","regeocode":example_response()}).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let settings = GeocodingSettings {
            provider: GeocodingProvider::Amap,
            api_key: "test-secret".into(),
            ..Default::default()
        };
        let key = request_key(&exif(), &settings).unwrap();
        assert_ne!(key.1, "116.310000,39.991000");
        assert_eq!(request_at(&key, &url).unwrap(), example_response());
        server.join().unwrap();
        assert!(!format!("{settings:?}").contains("test-secret"));
        let error = request_at(&key, "http://127.0.0.1:1")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("test-secret"));
        assert!(!error.contains("key="));
    }
    #[test]
    fn cached_responses_are_shared_while_keys_and_coordinates_are_isolated() {
        let settings = GeocodingSettings {
            provider: GeocodingProvider::Amap,
            api_key: "cached-fixture-key".into(),
            ..Default::default()
        };
        let source = exif();
        let key = request_key(&source, &settings).unwrap();
        RESPONSES.lock().unwrap().insert(
            key.clone(),
            CachedResponse {
                at: Instant::now(),
                value: Ok(example_response()),
            },
        );
        let groups = group("{格式化地址}");
        let first = resolve_exif(Some(&source), &settings, &groups)
            .unwrap()
            .unwrap();
        assert_eq!(
            first.address_fields,
            cached_exif(&source, &settings).address_fields
        );
        let mut changed = settings.clone();
        changed.api_key = "different-key".into();
        assert_eq!(
            cached_exif(&source, &changed).address_fields.unwrap()["格式化地址"],
            ""
        );
        let another = ExifInfo {
            gps_info: Some(Location::new(31.2, 121.4).unwrap().to_gps().unwrap()),
            ..Default::default()
        };
        assert_eq!(
            cached_exif(&another, &settings).address_fields.unwrap()["格式化地址"],
            ""
        );
        // UI 缓存读取不能被后台正在进行的 HTTP 请求阻塞。
        let guard = RESPONSES.lock().unwrap();
        assert_eq!(
            cached_exif(&source, &settings).address_fields.unwrap()["格式化地址"],
            ""
        );
        drop(guard);
        RESPONSES.lock().unwrap().remove(&key);
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    #[test]
    fn amap_templates_match_layered_preview_and_keep_hdr_gainmap() {
        use crate::{
            image::{PreviewJob, layers::PreviewLayerCache},
            photo::Photo,
            watermark::WatermarkParams,
        };
        let settings = GeocodingSettings {
            provider: GeocodingProvider::Amap,
            api_key: "rendering-fixture-key".into(),
            ..Default::default()
        };
        let exif = ExifInfo {
            gps_info: Some(Location::new(39.991, 116.31).unwrap().to_gps().unwrap()),
            ..Default::default()
        };
        let key = request_key(&exif, &settings).unwrap();
        RESPONSES.lock().unwrap().insert(
            key.clone(),
            CachedResponse {
                at: Instant::now(),
                value: Ok(example_response()),
            },
        );
        let mut group = TextGroup::default();
        group.text.template = vec!["{省|市|区|街道} {兴趣点0名称}".into()];
        group.text.text_params.truncate(1);
        let job = PreviewJob {
            path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../test_images/ultra_hdr.jpg"),
            exif: Some(exif),
            params: WatermarkParams {
                geocoding: settings,
                ..Default::default()
            },
            text_groups: vec![group],
            max_edge: 320,
        };
        let full = crate::image::render_preview(&job).unwrap();
        let layered = PreviewLayerCache::default().render(&job).unwrap();
        assert_eq!(layered.text_regions().len(), 1);
        assert_eq!((full.width() as i32, full.height() as i32), layered.size());
        let error = full
            .pixels()
            .iter()
            .zip(layered.image().pixels())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(error <= 2, "高德模板的完整合成与分层预览应一致：{error}");
        let base = crate::image::load_scaled(&job.path, job.max_edge).unwrap();
        let composed =
            Photo::compose_watermark(base, job.exif.as_ref(), &job.params, &job.text_groups)
                .unwrap();
        assert!(crate::gainmap::get_gainmap(&composed).is_some());
        RESPONSES.lock().unwrap().remove(&key);
    }
}
