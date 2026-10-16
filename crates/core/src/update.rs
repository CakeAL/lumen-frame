//! GitHub Releases 自更新。
//!
//! 网络请求和可执行文件替换都是阻塞操作；本模块保持无 UI 依赖，由界面层放到后台线程调用。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use self_update::backends::github;
use self_update::check_interval::UpdateCheckGuard;

const REPO_OWNER: &str = "CakeAL";
const REPO_NAME: &str = "lumen-frame";
const BIN_NAME: &str = "lumen-frame";

/// 自动检查更新的默认间隔。
pub const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateAvailability {
    UpToDate,
    Available { version: String },
}

/// 发布构建每七天自动检查一次；开发和测试构建避免在启动时意外访问网络。
pub fn automatic_checks_enabled() -> bool {
    !cfg!(debug_assertions)
}

/// 仅在距离上次成功检查已超过七天时访问 GitHub。
pub fn check_if_due() -> Result<Option<UpdateAvailability>> {
    let guard = update_check_guard()?;
    if !guard.should_check().context("无法读取上次更新检查时间")? {
        return Ok(None);
    }

    let availability = check_now()?;
    guard.record_check().context("无法记录更新检查时间")?;
    Ok(Some(availability))
}

/// 立即检查，并把成功检查的时间写入与自动检查共用的时间戳。
pub fn check_now_and_record() -> Result<UpdateAvailability> {
    let availability = check_now()?;
    update_check_guard()?
        .record_check()
        .context("无法记录更新检查时间")?;
    Ok(availability)
}

/// 安装 GitHub Releases 中适合当前平台的最新版本。
pub fn install_latest() -> Result<String> {
    let status = updater(true)?.update().context("下载或安装更新失败")?;
    Ok(status.version().to_string())
}

fn check_now() -> Result<UpdateAvailability> {
    let release = updater(false)?
        .is_update_available()
        .context("无法读取 GitHub Releases")?;
    Ok(match release {
        Some(release) => UpdateAvailability::Available {
            version: release.version().to_string(),
        },
        None => UpdateAvailability::UpToDate,
    })
}

fn updater(for_install: bool) -> Result<github::Update> {
    let mut builder = github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(env!("CARGO_PKG_VERSION"))
        // GUI 已经负责确认和反馈，不能让后台线程等待终端输入或输出进度条。
        .unattended()
        .show_download_progress(false)
        .check_install_path_writable(true);

    // 查询 Releases 不需要知道安装位置。只有真正安装时才启用完整 .app 替换；否则
    // `cargo run` 位于 target/debug，没有 `.app` 上级目录，构建更新器就会提前失败。
    #[cfg(target_os = "macos")]
    if for_install {
        ensure_running_from_app_bundle()?;
        builder.bundle_path_in_archive("Lumen Frame.app");
    }

    #[cfg(not(target_os = "macos"))]
    let _ = for_install;

    builder.build().context("更新器配置无效")
}

#[cfg(target_os = "macos")]
fn ensure_running_from_app_bundle() -> Result<()> {
    let executable = std::env::current_exe().context("无法确定当前程序路径")?;
    if executable
        .ancestors()
        .any(|path| path.extension().is_some_and(|extension| extension == "app"))
    {
        return Ok(());
    }

    anyhow::bail!("开发构建只能检查更新；请从打包后的 Lumen Frame.app 中安装更新")
}

fn update_check_guard() -> Result<UpdateCheckGuard> {
    let cache_dir = dirs::cache_dir()
        .context("找不到系统缓存目录")?
        .join("lumen-frame");
    update_check_guard_in(&cache_dir)
}

fn update_check_guard_in(cache_dir: &Path) -> Result<UpdateCheckGuard> {
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("无法创建更新缓存目录 {}", cache_dir.display()))?;
    Ok(UpdateCheckGuard::new(
        update_stamp_path(cache_dir),
        DEFAULT_CHECK_INTERVAL,
    ))
}

fn update_stamp_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("update-check.stamp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_check_interval_is_seven_days() {
        assert_eq!(DEFAULT_CHECK_INTERVAL, Duration::from_secs(604_800));
    }

    #[test]
    fn stamp_lives_inside_the_app_cache_directory() {
        let cache_dir = Path::new("cache").join("lumen-frame");
        assert_eq!(
            update_stamp_path(&cache_dir),
            cache_dir.join("update-check.stamp")
        );
    }

    #[test]
    fn checking_does_not_require_an_app_bundle() {
        updater(false).expect("checking should work from cargo test/target/debug");
    }
}
