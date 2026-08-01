//! CDP 导航状态事件解析。

use std::time::Duration;

use wisp_browser::cdp::CdpEvent;
use wisp_core::error::{BrowserError, Result, WispError};

fn is_document_event(event: &CdpEvent) -> bool {
    event.params.get("type").and_then(|t| t.as_str()) == Some("Document")
}

fn loading_failed_error(event: &CdpEvent) -> Option<WispError> {
    if event.method != "Network.loadingFailed" || !is_document_event(event) {
        return None;
    }
    let error_text = event
        .params
        .get("errorText")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown");
    tracing::warn!("recv_navigation_status: Network.loadingFailed errorText={error_text}");
    Some(WispError::Browser(BrowserError::CdpConnection(format!(
        "navigation loading failed: {error_text}"
    ))))
}

fn response_status_from_event(event: &CdpEvent) -> Result<u16> {
    event
        .params
        .get("response")
        .and_then(|r| r.get("status"))
        .and_then(serde_json::Value::as_u64)
        .map(|s| s as u16)
        .ok_or_else(|| {
            WispError::Browser(BrowserError::CdpConnection(
                "Network.responseReceived missing response.status".into(),
            ))
        })
}

/// 从事件流中接收 `Network.responseReceived` (type=Document) 事件并提取状态码。
///
/// 必须在 `goto` 之前订阅 `event_rx`，否则可能丢失事件。
/// 5s 超时：导航通常在 1-3s 内完成，5s 足够覆盖慢速页面。
///
/// 特殊处理：若先收到 `Network.loadingFailed` (type=Document)，说明导航请求失败
///（如代理连接失败、DNS 解析失败），立即返回错误，不空等 5s 超时。
pub(crate) async fn recv_navigation_status(
    rx: &mut tokio::sync::broadcast::Receiver<CdpEvent>,
    sid: &str,
) -> Result<u16> {
    use tokio::sync::broadcast::error::RecvError;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(event)) => {
                if event.session_id.as_deref() != Some(sid) && event.session_id.is_some() {
                    continue;
                }
                if let Some(err) = loading_failed_error(&event) {
                    return Err(err);
                }
                if event.method != "Network.responseReceived" {
                    continue;
                }
                if !is_document_event(&event) {
                    continue;
                }
                return response_status_from_event(&event);
            }
            Ok(Err(RecvError::Lagged(n))) => {
                tracing::warn!("event subscriber lagged by {n} events, continuing recv");
            }
            Ok(Err(RecvError::Closed)) => {
                return Err(WispError::Browser(BrowserError::CdpConnection(
                    "event broadcaster closed before navigation status captured".into(),
                )));
            }
            Err(_) => {
                tracing::warn!(
                    "capture_navigation_status: 5s 内未收到 Network.responseReceived，返回默认 200"
                );
                return Ok(200);
            }
        }
    }
}
