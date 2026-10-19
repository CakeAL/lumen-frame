//! 本机 Logo 素材库。文件保留稳定编号，自定义名称另存元数据；删除后不复用编号。
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, OnceLock},
};

const NIKON: &[u8] = include_bytes!("../../../../assets/logo/examples/nikon.svg");
const NIKON_NAME: &str = "尼康logo";
const MANIFEST: &str = "logos.toml";

#[derive(Clone)]
pub struct LogoAsset {
    id: u64,
    name: String,
    path: Option<PathBuf>,
    bytes: Arc<Vec<u8>>,
}
impl LogoAsset {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }
    pub fn name(&self) -> String {
        self.name.clone()
    }
    pub fn token(&self) -> String {
        format!("{{{}}}", self.name)
    }
    pub fn is_builtin(&self) -> bool {
        self.path.is_none()
    }
}

/// 一次导入中每个文件各自的名称；省略名称时沿用「自定义logoN」。
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogoImport {
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
}
impl LogoImport {
    pub fn new(path: PathBuf, name: Option<String>) -> Self {
        Self { path, name }
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Manifest {
    next_id: u64,
    names: BTreeMap<String, String>,
}
impl Manifest {
    fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(MANIFEST);
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).context("Logo 名称记录损坏，请检查 logos.toml"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).context("读取 Logo 名称记录失败"),
        }
    }
    fn save(&self, dir: &Path) -> Result<()> {
        let temporary = dir.join("logos.toml.tmp");
        std::fs::write(&temporary, toml::to_string_pretty(self)?)?;
        std::fs::rename(temporary, dir.join(MANIFEST)).context("保存 Logo 名称记录失败")
    }
    fn reserve_after(&mut self, assets: &[LogoAsset]) -> Result<u64> {
        let last = assets.iter().map(LogoAsset::id).max().unwrap_or(0);
        let next = last
            .checked_add(1)
            .context("Logo 编号已用完")?
            .max(self.next_id)
            .max(1);
        self.next_id = next;
        Ok(next)
    }
}
fn lock(dir: &Path) -> Result<File> {
    std::fs::create_dir_all(dir).context("创建自定义 Logo 目录失败")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    file.try_lock().context("素材库正在更新，请稍后重试")?;
    Ok(file)
}
fn legacy_name(id: u64) -> String {
    format!("自定义logo{id}")
}
fn legacy_id(name: &str) -> Option<u64> {
    name.strip_prefix("自定义logo")?
        .parse()
        .ok()
        .filter(|id| *id > 0)
}

/// 自定义字段约定：默认「自定义logoN」，命名后为「名称logo」。
pub fn is_logo_field(name: &str) -> bool {
    legacy_id(name).is_some() || (name.ends_with("logo") && name.len() > 4)
}
/// 供渲染缓存和编辑器共用，保留默认编号与已删除素材的命名字段。
pub fn template_logo_names(template: &str) -> impl Iterator<Item = &str> {
    static FIELDS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\{([^{}\r\n]+)\}").unwrap());
    FIELDS
        .captures_iter(template)
        .filter_map(|captures| captures.get(1).map(|name| name.as_str()))
        .filter(|name| is_logo_field(name))
}
/// 输入「尼康」和「尼康logo」都会得到「尼康logo」。名称仅作元数据，不参与路径拼接。
pub fn custom_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= 64,
        "Logo 名称需为 1–64 个字符"
    );
    ensure!(
        !name
            .chars()
            .any(|ch| matches!(ch, '{' | '}') || ch.is_control()),
        "Logo 名称不能包含大括号或控制字符"
    );
    ensure!(
        !name.starts_with("自定义logo"),
        "「自定义logo」前缀用于默认编号，请使用其他名称"
    );
    let suffix = name.get(name.len().saturating_sub(4)..);
    let name = if suffix.is_some_and(|suffix| suffix.eq_ignore_ascii_case("logo")) {
        ensure!(name.len() > 4, "请在 logo 前填写名称");
        format!("{}logo", name[..name.len() - 4].trim_end())
    } else {
        format!("{name}logo")
    };
    Ok(name)
}

pub fn directory() -> Option<PathBuf> {
    dirs::data_local_dir().map(|root| root.join("lumen-frame").join("logos"))
}
pub fn builtin_bytes(name: &str) -> Option<&'static Arc<Vec<u8>>> {
    static BYTES: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
    (name == NIKON_NAME).then(|| BYTES.get_or_init(|| Arc::new(NIKON.to_vec())))
}
pub fn builtin() -> Vec<LogoAsset> {
    vec![LogoAsset {
        id: 0,
        name: NIKON_NAME.into(),
        path: None,
        bytes: builtin_bytes(NIKON_NAME).unwrap().clone(),
    }]
}
/// 桌面和 MCP 展示的完整素材库；内置示例不写入用户目录。
pub fn library_in(dir: &Path) -> Result<Vec<LogoAsset>> {
    let mut library = builtin();
    library.extend(load_in(dir)?);
    Ok(library)
}
/// 兼容早期只有「自定义logoN.ext」的素材目录。
pub fn load_in(dir: &Path) -> Result<Vec<LogoAsset>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let manifest = Manifest::load(dir)?;
    let mut assets = Vec::new();
    for entry in std::fs::read_dir(dir).context("读取自定义 Logo 目录失败")? {
        let path = entry?.path();
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(legacy_id)
        else {
            continue;
        };
        if !supported(&path) || !path.is_file() {
            continue;
        }
        let bytes =
            std::fs::read(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
        assets.push(LogoAsset {
            id,
            name: manifest
                .names
                .get(&id.to_string())
                .cloned()
                .unwrap_or_else(|| legacy_name(id)),
            path: Some(path),
            bytes: Arc::new(bytes),
        });
    }
    assets.sort_by_key(LogoAsset::id);
    Ok(assets)
}
pub fn supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "svg" | "jpg" | "jpeg" | "webp"
        )
    })
}
pub fn import_in(dir: &Path, paths: &[PathBuf]) -> Result<Vec<LogoAsset>> {
    import_named_in(
        dir,
        &paths
            .iter()
            .map(|path| LogoImport::new(path.clone(), None))
            .collect::<Vec<_>>(),
    )
}
/// 整批验证格式、名称和重复项后写入；空名称使用默认编号。
pub fn import_named_in(dir: &Path, files: &[LogoImport]) -> Result<Vec<LogoAsset>> {
    let _lock = lock(dir)?;
    let mut assets = load_in(dir)?;
    let mut manifest = Manifest::load(dir)?;
    let mut next_id = manifest.reserve_after(&assets)?;
    let mut names = assets
        .iter()
        .map(LogoAsset::name)
        .chain(builtin().iter().map(LogoAsset::name))
        .collect::<HashSet<_>>();
    let mut prepared = Vec::new();
    for file in files {
        ensure!(
            supported(&file.path),
            "请选择 PNG、SVG、JPEG 或 WebP 文件：{}",
            file.path.display()
        );
        let name = file
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(custom_name)
            .transpose()?
            .unwrap_or_else(|| legacy_name(next_id));
        ensure!(
            names.insert(name.clone()),
            "Logo 名称「{name}」已存在，请换一个名称"
        );
        let bytes = std::fs::read(&file.path)
            .with_context(|| format!("读取 {} 失败", file.path.display()))?;
        crate::media::load_logo_image(&bytes)
            .with_context(|| format!("无法读取 Logo：{}", file.path.display()))?;
        let ext = file
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap()
            .to_ascii_lowercase();
        prepared.push((next_id, name, bytes, ext));
        next_id = next_id.checked_add(1).context("Logo 编号已用完")?;
    }
    // 先记录高水位；即便进程在写文件时退出，也不能让旧模板引用到另一张图。
    manifest.next_id = next_id;
    manifest.save(dir)?;
    for (id, name, bytes, ext) in prepared {
        let path = dir.join(format!("{}.{}", legacy_name(id), ext));
        use std::io::Write as _;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .context("保存 Logo 失败")?;
        file.write_all(&bytes).context("写入 Logo 失败")?;
        manifest.names.insert(id.to_string(), name.clone());
        manifest.save(dir)?;
        assets.push(LogoAsset {
            id,
            name,
            path: Some(path),
            bytes: Arc::new(bytes),
        });
    }
    Ok(assets)
}
/// 删除素材库副本，保留源文件和模板文字；旧编号不会指向后续导入的素材。
pub fn delete_in(dir: &Path, id: u64) -> Result<Vec<LogoAsset>> {
    ensure!(id != 0, "内置示例不能删除");
    let _lock = lock(dir)?;
    let assets = load_in(dir)?;
    let asset = assets
        .iter()
        .find(|asset| asset.id == id)
        .context("Logo 素材不存在")?;
    let mut manifest = Manifest::load(dir)?;
    manifest.reserve_after(&assets)?;
    manifest.save(dir)?;
    std::fs::remove_file(asset.path().context("内置示例不能删除")?)
        .context("删除 Logo 素材失败")?;
    manifest.names.remove(&id.to_string());
    manifest.save(dir)?;
    load_in(dir)
}
pub fn snapshot(assets: &[LogoAsset]) -> BTreeMap<String, Arc<Vec<u8>>> {
    let mut snapshot = BTreeMap::from([(
        NIKON_NAME.into(),
        builtin_bytes(NIKON_NAME).unwrap().clone(),
    )]);
    for asset in assets {
        snapshot.insert(asset.name(), asset.bytes.clone());
        if !asset.is_builtin() {
            snapshot.insert(legacy_name(asset.id), asset.bytes.clone());
        }
    }
    snapshot
}
