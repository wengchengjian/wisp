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
pub(crate) use locate::{find_installed_browser, get_install_root};

use download::download_and_install;
use std::path::PathBuf;
use version::get_latest_version;
use wisp_core::error::Result;

pub async fn ensure_browser_installed() -> Result<PathBuf> {
    let install_root = get_install_root();

    // 检查是否已安装
    if let Some(path) = find_installed_browser(&install_root)? {
        tracing::debug!("Chrome for Testing 已安装: {}", path.display());
        return Ok(path);
    }

    // 下载安装
    tracing::info!("未检测到浏览器，开始下载 Chrome for Testing...");
    let version = get_latest_version().await?;
    let path = download_and_install(&version, &install_root).await?;
    tracing::info!(
        "Chrome for Testing {} 安装完成: {}",
        version,
        path.display()
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::locate::find_executable_in_dir;
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
