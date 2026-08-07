//! Chrome for Testing 自动安装器。
//!
//! 当系统未安装 Chrome/Chromium/Edge 时，自动从 Google 官方下载 Chrome for Testing。
//! 安装到项目本地 `.browsers/` 目录，避免需要 root 权限。
//!
//! # 安装位置
//! `{cwd}/.browsers/chrome-{version}/chrome-{platform}/chrome`
//!
//! # 平台支持
//! - Linux x86_64/aarch64 → `linux64`
//! - macOS x86_64 → `mac-x64`
//! - macOS aarch64 → `mac-arm64`
//! - Windows x86_64 → `win64`

mod download;
mod extract;
mod locate;
mod version;

pub use locate::get_platform_id;
pub(crate) use locate::{
    find_executable_in_dir, find_installed_browser_for_major, get_install_root,
};

use download::download_and_install;
use std::path::PathBuf;
use version::{get_latest_version, get_version_for_major};
use wisp_core::error::Result;

/// 固定的 Chrome for Testing 主版本。
///
/// 与 CF 快速路径的 TLS 指纹档位（`Chrome148`，见 wisp-fetcher select_cf_profile）
/// 对齐，保证浏览器与 HTTP 指纹一致。固定版本避免版本漂移导致的指纹不自洽
/// 与排查困难；同时 `Chrome149` 档位经实测与 CF 复用不兼容（403）。
/// 148 仍在 Google Cloud Storage 提供（可下载），不要设更低（旧版本会被移除）。
pub const SUPPORTED_CHROME_MAJOR: u32 = 148;

/// 下载并安装指定版本的 Chrome for Testing。
///
/// `version` 形如 `149.0.0.0`（Chrome for Testing 版本号）。安装目录带版本号，
/// 因此不同版本可共存，重复调用不会重复下载同一版本。
async fn ensure_browser_installed_version(version: &str) -> Result<PathBuf> {
    let install_root = get_install_root();

    // 检查该版本是否已安装
    let version_dir = install_root.join(format!("chrome-{version}"));
    if version_dir.exists()
        && let Some(path) = find_executable_in_dir(&version_dir)?
    {
        tracing::debug!("Chrome for Testing {version} 已安装: {}", path.display());
        return Ok(path);
    }

    // 下载安装
    tracing::info!("开始下载 Chrome for Testing {version}...");
    let path = download_and_install(version, &install_root).await?;
    tracing::info!("Chrome for Testing {version} 安装完成: {}", path.display());
    Ok(path)
}

/// 自动下载安装浏览器：固定使用 `SUPPORTED_CHROME_MAJOR`（Chrome 148）。
///
/// 固定版本与 CF 快速路径 TLS 指纹档位（`Chrome148`）对齐，避免版本漂移导致的
/// 指纹不自洽与排查困难。优先复用 `.browsers/` 中已安装的同主版本，避免重新下载
/// 该主版本下的其他小版本；查询/下载失败时回退到最新版。
pub async fn ensure_browser_installed() -> Result<PathBuf> {
    let major = SUPPORTED_CHROME_MAJOR;
    let install_root = get_install_root();

    // 优先复用已安装的固定主版本（148），避免重新下载该主版本下的其他小版本。
    if let Some(path) = find_installed_browser_for_major(&install_root, major)? {
        tracing::debug!("复用已安装的 Chrome {major}: {}", path.display());
        return Ok(path);
    }

    match get_version_for_major(major).await {
        Ok(version) => ensure_browser_installed_version(&version).await,
        Err(e) => {
            tracing::warn!("获取 Chrome {major} 版本失败: {e}，回退到最新版");
            let version = get_latest_version().await?;
            ensure_browser_installed_version(&version).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::locate::{find_executable_in_dir, find_installed_browser};
    use super::*;
    use std::path::Path;

    #[test]
    fn test_get_platform_id_not_empty() {
        let platform = get_platform_id();
        assert!(!platform.is_empty(), "平台标识不应为空");
        // 应该是已知平台之一
        assert!(
            matches!(
                platform,
                "linux64" | "mac-x64" | "mac-arm64" | "win64" | "win32"
            ),
            "未知平台: {platform}"
        );
    }

    #[test]
    fn test_get_install_root_ends_with_browsers() {
        let root = get_install_root();
        assert!(
            root.ends_with(".browsers"),
            "安装根目录应以 .browsers 结尾: {}",
            root.display()
        );
    }

    #[test]
    fn test_find_installed_browser_empty_dir() {
        let temp = tempfile::tempdir().unwrap();
        let result = find_installed_browser(temp.path()).unwrap();
        assert!(result.is_none(), "空目录应返回 None");
    }

    #[test]
    fn test_find_installed_browser_no_browsers() {
        let temp = tempfile::tempdir().unwrap();
        // 创建一些无关文件
        std::fs::write(temp.path().join("readme.txt"), "hello").unwrap();
        let result = find_installed_browser(temp.path()).unwrap();
        assert!(result.is_none(), "无浏览器目录应返回 None");
    }

    #[test]
    fn test_find_executable_in_nonexistent_dir() {
        let result = find_executable_in_dir(Path::new("/nonexistent/path")).unwrap();
        assert!(result.is_none(), "不存在的目录应返回 None");
    }

    #[tokio::test]
    async fn test_get_latest_version() {
        // 网络测试：可能因网络环境失败
        let result = get_latest_version().await;
        if let Ok(version) = result {
            assert!(version.contains('.'), "版本号应包含点号: {version}");
            tracing::info!("最新 Chrome for Testing 版本: {version}");
        } else {
            // 网络不可用时跳过
            eprintln!("跳过：网络不可用");
        }
    }
}
