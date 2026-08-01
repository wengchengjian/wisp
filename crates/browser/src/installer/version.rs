//! Chrome for Testing 版本查询。

use wisp_core::error::{BrowserError, Result, WispError};

/// 最新稳定版版本号 JSON API
const LATEST_VERSION_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json";

pub(super) async fn get_latest_version() -> Result<String> {
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
