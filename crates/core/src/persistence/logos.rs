//! 自定义 Logo 素材库。原始文件保存在本机 support 目录，编号不会因添加而改变。

use anyhow::{Context as _, Result, bail};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct LogoAsset {
    id: u64,
    path: PathBuf,
    bytes: Arc<Vec<u8>>,
}

impl LogoAsset {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    pub fn name(&self) -> String {
        format!("自定义logo{}", self.id)
    }
    pub fn token(&self) -> String {
        format!("{{{}}}", self.name())
    }
}

pub fn directory() -> Option<PathBuf> {
    dirs::data_local_dir().map(|root| root.join("lumen-frame").join("logos"))
}

pub fn load_in(dir: &Path) -> Result<Vec<LogoAsset>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut assets = Vec::new();
    for entry in std::fs::read_dir(dir).context("读取自定义 Logo 目录失败")? {
        let path = entry?.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(id) = stem
            .strip_prefix("自定义logo")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|id| *id > 0)
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
            path,
            bytes: Arc::new(bytes),
        });
    }
    assets.sort_by_key(|asset| asset.id);
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
    std::fs::create_dir_all(dir).context("创建自定义 Logo 目录失败")?;
    let mut assets = load_in(dir)?;
    let mut next_id = assets.iter().map(|asset| asset.id).max().unwrap_or(0) + 1;
    // 先验证整批文件，失败不会留下半批导入记录。
    let mut prepared = Vec::new();
    for path in paths {
        if !supported(path) {
            bail!("请选择 PNG、SVG、JPEG 或 WebP 文件：{}", path.display());
        }
        let bytes = std::fs::read(path).with_context(|| format!("读取 {} 失败", path.display()))?;
        crate::media::load_logo_image(&bytes)
            .with_context(|| format!("无法读取 Logo：{}", path.display()))?;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap()
            .to_ascii_lowercase();
        prepared.push((bytes, ext));
    }
    for (bytes, ext) in prepared {
        let path = dir.join(format!("自定义logo{next_id}.{ext}"));
        // 不覆盖已有素材，模板编号一直指向同一份内容。
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .context("保存 Logo 失败")?;
        file.write_all(&bytes).context("写入 Logo 失败")?;
        assets.push(LogoAsset {
            id: next_id,
            path,
            bytes: Arc::new(bytes),
        });
        next_id += 1;
    }
    Ok(assets)
}

pub fn snapshot(assets: &[LogoAsset]) -> BTreeMap<String, Arc<Vec<u8>>> {
    assets
        .iter()
        .map(|asset| (asset.name(), asset.bytes.clone()))
        .collect()
}
