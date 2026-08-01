//! wreq 错误分类。

use std::time::Duration;

use wisp_core::error::{NetworkError, WispError};

/// 将 `wreq::Error` 分类为结构化的 `WispError`，支持按错误类别差异化重试/降级。
///
/// 分类顺序（先命中先返回）：
/// 1. `is_timeout` → `RequestTimedOut`
/// 2. `is_proxy_connect` → `ProxyFailed`
/// 3. `is_tls` → `TlsFailed`
/// 4. `is_connect` → 区分 DNS 失败（`DnsFailed`）与 TCP 连接失败（`ConnectionFailed`）
/// 5. `is_builder` → `UrlParse`（通常是 URL scheme/解析错误）
/// 6. 其他 → 退化为 `HttpError`
///
/// DNS 判定通过错误消息字符串匹配（wreq 未暴露 DNS 专用判断 API），覆盖常见
/// GaiError 消息："nodename nor servname"、"name or service not known"、
/// "no such host"、"name resolution" 等。
pub(super) fn classify_request_error(e: &wreq::Error, url: &str, timeout: Duration) -> WispError {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| url.to_string());

    if e.is_timeout() {
        return WispError::Network(NetworkError::RequestTimedOut {
            url: url.to_string(),
            timeout_secs: timeout.as_secs(),
        });
    }
    if e.is_proxy_connect() {
        return WispError::Network(NetworkError::ProxyFailed {
            detail: e.to_string(),
        });
    }
    if e.is_tls() {
        return WispError::Network(NetworkError::TlsFailed {
            host,
            detail: e.to_string(),
        });
    }
    if e.is_connect() {
        let detail = e.to_string();
        let lower = detail.to_lowercase();
        const DNS_HINTS: &[&str] = &[
            "nodename nor servname",
            "name or service not known",
            "no such host",
            "name resolution",
            "dns",
            "resolve",
            "temporary failure in name resolution",
        ];
        if DNS_HINTS.iter().any(|h| lower.contains(h)) {
            return WispError::Network(NetworkError::DnsFailed { host, detail });
        }
        return WispError::Network(NetworkError::ConnectionFailed { host, detail });
    }
    if e.is_builder() {
        return WispError::Network(NetworkError::UrlParse(e.to_string()));
    }
    WispError::Network(NetworkError::Http(e.to_string()))
}
