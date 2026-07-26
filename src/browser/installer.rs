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

use std::path::{Path, PathBuf};

use crate::error::{BrowserError, Result, WispError};

/// 最新稳定版版本号 JSON API
const LATEST_VERSION_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json";

/// 下载 URL 模板
const DOWNLOAD_URL_TEMPLATE: &str =
    "https://storage.googleapis.com/chrome-for-testing-public/{version}/{platform}/chrome-{platform}.zip";

/// 确保 Chrome for Testing 已安装。若已安装则返回路径，否则下载安装。
///
/// 安装位置：`{cwd}/.browsers/chrome-{version}/`
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

/// 获取安装根目录：`{cwd}/.browsers/`
fn get_install_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".browsers")
}

/// 在安装目录中查找已安装的浏览器
fn find_installed_browser(install_root: &Path) -> Result<Option<PathBuf>> {
    if !install_root.exists() {
        return Ok(None);
    }

    // 遍历 .browsers/chrome-*/ 目录
    for entry in std::fs::read_dir(install_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(exe) = find_executable_in_dir(&path)? {
                return Ok(Some(exe));
            }
        }
    }
    Ok(None)
}

/// 在版本目录中查找可执行文件
fn find_executable_in_dir(version_dir: &Path) -> Result<Option<PathBuf>> {
    let platform = get_platform_id();
    let chrome_dir = version_dir.join(format!("chrome-{platform}"));

    let exe_rel = if cfg!(target_os = "windows") {
        "chrome.exe"
    } else if cfg!(target_os = "macos") {
        "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
    } else {
        "chrome"
    };

    let exe_path = chrome_dir.join(exe_rel);
    if exe_path.exists() {
        return Ok(Some(exe_path));
    }

    Ok(None)
}

/// 获取当前平台的标识
#[must_use]
pub fn get_platform_id() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "mac-x64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "mac-arm64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "win64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86"))]
    {
        "win32"
    }
}

/// 从 JSON API 获取最新稳定版版本号
async fn get_latest_version() -> Result<String> {
    let client = wreq::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!(
                "创建 HTTP 客户端失败: {e}"
            )))
        })?;

    let resp = client.get(LATEST_VERSION_URL).send().await.map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "获取 Chrome for Testing 版本信息失败: {e}"
        )))
    })?;

    let body = resp.text().await.map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "读取版本信息响应失败: {e}"
        )))
    })?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "解析版本信息 JSON 失败: {e}"
        )))
    })?;

    let version = json["channels"]["Stable"]["version"]
        .as_str()
        .ok_or_else(|| {
            WispError::Browser(BrowserError::LaunchFailed(
                "无法解析 Chrome for Testing 版本号".into(),
            ))
        })?;

    Ok(version.to_string())
}

/// 下载并安装 Chrome for Testing
async fn download_and_install(version: &str, install_root: &Path) -> Result<PathBuf> {
    let platform = get_platform_id();
    let url = DOWNLOAD_URL_TEMPLATE
        .replace("{version}", version)
        .replace("{platform}", platform);

    let version_dir = install_root.join(format!("chrome-{version}"));
    std::fs::create_dir_all(&version_dir)?;

    // 下载 zip 文件
    tracing::info!("下载 Chrome for Testing {version} ({platform})");
    let client = wreq::Client::builder()
        .timeout(std::time::Duration::from_mins(5))
        .build()
        .map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!(
                "创建下载客户端失败: {e}"
            )))
        })?;

    let resp =
        client.get(&url).send().await.map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!("下载失败: {e}")))
        })?;

    if !resp.status().is_success() {
        return Err(WispError::Browser(BrowserError::LaunchFailed(format!(
            "下载 Chrome for Testing 失败: HTTP {}",
            resp.status()
        ))));
    }

    // 流式下载到临时文件
    let temp_zip = install_root.join(format!("chrome-{version}.zip"));
    let total_size = resp.content_length().unwrap_or(0);
    if total_size > 0 {
        tracing::info!("文件大小: {} MB", total_size / 1024 / 1024);
    }

    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&temp_zip).await.map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!("创建临时文件失败: {e}")))
    })?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!("下载流读取失败: {e}")))
        })?;
        file.write_all(&chunk).await.map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!("写入临时文件失败: {e}")))
        })?;
    }
    file.flush()
        .await
        .map_err(|e| WispError::Browser(BrowserError::LaunchFailed(format!("flush 失败: {e}"))))?;
    drop(file);

    // 解压
    tracing::info!("解压到: {}", version_dir.display());
    extract_zip(&temp_zip, &version_dir)?;

    // 删除 zip 文件
    let _ = std::fs::remove_file(&temp_zip);

    // 设置可执行权限（Linux/macOS）
    // 对整个 chrome-{platform}/ 目录递归设置 0o755，确保 chrome、chrome_crashpad_handler
    // 等所有可执行文件都有执行权限（zip 可能不保留 Unix 权限信息）
    let exe_path = find_executable_in_dir(&version_dir)?.ok_or_else(|| {
        WispError::Browser(BrowserError::LaunchFailed(
            "解压后未找到 Chrome 可执行文件".into(),
        ))
    })?;

    #[cfg(unix)]
    {
        fix_permissions(&version_dir)?;
    }

    Ok(exe_path)
}

/// 递归设置目录下所有文件的可执行权限（0o755）。
///
/// Chrome 目录下的可执行文件（chrome、chrome_crashpad_handler）和共享库（.so）
/// 都需要可执行权限。zip 解压可能不保留原始权限，需要手动修复。
#[cfg(unix)]
fn fix_permissions(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fn walk(dir: &Path) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(
                    |e| {
                        WispError::Browser(BrowserError::LaunchFailed(format!(
                            "设置目录权限失败: {e}"
                        )))
                    },
                )?;
                walk(&path)?;
            } else {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(
                    |e| {
                        WispError::Browser(BrowserError::LaunchFailed(format!(
                            "设置文件权限失败: {e}"
                        )))
                    },
                )?;
            }
        }
        Ok(())
    }

    walk(dir)
}

/// 解压 zip 文件到目标目录
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "打开 zip 文件失败: {e}"
        )))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "读取 zip 归档失败: {e}"
        )))
    })?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!(
                "读取 zip 条目 {i} 失败: {e}"
            )))
        })?;
        let name = entry.name().to_string();
        let outpath = dest.join(&name);

        if entry.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| {
                WispError::Browser(BrowserError::LaunchFailed(format!("创建目录失败: {e}")))
            })?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    WispError::Browser(BrowserError::LaunchFailed(format!("创建父目录失败: {e}")))
                })?;
            }
            let mut outfile = std::fs::File::create(&outpath).map_err(|e| {
                WispError::Browser(BrowserError::LaunchFailed(format!("创建文件失败: {e}")))
            })?;
            std::io::copy(&mut entry, &mut outfile).map_err(|e| {
                WispError::Browser(BrowserError::LaunchFailed(format!("写入文件失败: {e}")))
            })?;
            drop(outfile);

            // 保留 zip 中的 Unix 权限（chrome、chrome_crashpad_handler 等需要可执行权限）
            #[cfg(unix)]
            {
                let mode = entry.unix_mode();
                if let Some(mode) = mode {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
