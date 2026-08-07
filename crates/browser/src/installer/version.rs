//! Chrome for Testing 版本查询。

use wisp_core::error::{BrowserError, Result, WispError};

/// 最新稳定版版本号 JSON API
const LATEST_VERSION_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json";

/// 所有已知良好版本列表 JSON API（用于按主版本号查询完整版本）
const KNOWN_GOOD_VERSIONS_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/known-good-versions.json";

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

/// 查询指定主版本号（如 149）的最新完整版本号（如 `149.0.5828.0`）。
///
/// 从 `known-good-versions.json` 遍历 `versions` 数组，取主版本匹配、版本号最大的一项。
/// 网络不可用时返回 `Err`（调用方可回退到最新版）。
pub(super) async fn get_version_for_major(major: u32) -> Result<String> {
    let client = wreq::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!(
                "创建 HTTP 客户端失败: {e}"
            )))
        })?;

    let resp = client
        .get(KNOWN_GOOD_VERSIONS_URL)
        .send()
        .await
        .map_err(|e| {
            WispError::Browser(BrowserError::LaunchFailed(format!(
                "获取 Chrome for Testing 版本列表失败: {e}"
            )))
        })?;

    let body = resp.text().await.map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "读取版本列表响应失败: {e}"
        )))
    })?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "解析版本列表 JSON 失败: {e}"
        )))
    })?;

    let versions = json["versions"].as_array().ok_or_else(|| {
        WispError::Browser(BrowserError::LaunchFailed(
            "无法解析 Chrome for Testing 版本列表".into(),
        ))
    })?;

    let mut best: Option<(u64, String)> = None;
    for item in versions {
        let Some(ver) = item["version"].as_str() else {
            continue;
        };
        // 解析主版本号：第一个点号前的数字
        let Some(major_str) = ver.split('.').next() else {
            continue;
        };
        let Ok(ver_major) = major_str.parse::<u32>() else {
            continue;
        };
        if ver_major != major {
            continue;
        }
        // 用构建号+补丁号比较，取该主版本下最新的
        let key: u64 = ver
            .split('.')
            .skip(1)
            .fold(0u64, |acc, s| acc * 10000 + s.parse().unwrap_or(0));
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, ver.to_string()));
        }
    }

    best.map(|(_, v)| v).ok_or_else(|| {
        WispError::Browser(BrowserError::LaunchFailed(format!(
            "未找到主版本 {major} 的 Chrome for Testing 版本"
        )))
    })
}
