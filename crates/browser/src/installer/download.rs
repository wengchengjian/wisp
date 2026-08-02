//! Chrome for Testing 下载。

use std::path::{Path, PathBuf};

use super::extract::extract_and_locate;
#[cfg(unix)]
use super::extract::fix_permissions;
use super::locate::get_platform_id;
use wisp_core::error::{BrowserError, Result, WispError};

/// 下载 URL 模板
const DOWNLOAD_URL_TEMPLATE: &str = "https://storage.googleapis.com/chrome-for-testing-public/{version}/{platform}/chrome-{platform}.zip";

async fn download_to_temp(version: &str, platform: &str, install_root: &Path) -> Result<PathBuf> {
    let url = DOWNLOAD_URL_TEMPLATE
        .replace("{version}", version)
        .replace("{platform}", platform);
    let client = wreq::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
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
    Ok(temp_zip)
}
/// 下载并安装 Chrome for Testing
pub(super) async fn download_and_install(version: &str, install_root: &Path) -> Result<PathBuf> {
    let platform = get_platform_id();
    let version_dir = install_root.join(format!("chrome-{version}"));
    std::fs::create_dir_all(&version_dir)?;
    tracing::info!("下载 Chrome for Testing {version} ({platform})");
    let temp_zip = download_to_temp(version, platform, install_root).await?;
    let exe_path = extract_and_locate(version, install_root, &temp_zip)?;
    #[cfg(unix)]
    {
        fix_permissions(&version_dir)?;
    }
    Ok(exe_path)
}
