//! Navigation, reload and load-state waiting.

use super::*;
use crate::cdp::CdpEvent;
use wisp_core::error::{BrowserError, Result, WispError};

impl Page {
    /// 导航到指定 URL，返回导航状态码。
    pub async fn goto(&mut self, url: &str) -> Result<u16> {
        do_goto(self, url).await
    }
    /// 重新加载当前页面。
    pub async fn reload(&self) -> Result<()> {
        do_reload(self).await
    }
    /// 后退（历史记录）。
    pub async fn go_back(&self) -> Result<()> {
        self.cmd(
            "Page.navigate",
            json!({ "url": "javascript:history.back()" }),
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }
    /// 前进（历史记录）。
    pub async fn go_forward(&self) -> Result<()> {
        self.cmd(
            "Page.navigate",
            json!({ "url": "javascript:history.forward()" }),
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }

    /// Wait for a specific URL pattern (substring match).
    pub async fn wait_for_url(&self, url_pattern: &str, timeout_ms: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let current = self.url().await?;
            if current.contains(url_pattern) {
                return Ok(());
            }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout(format!("wait_for_url: {url_pattern}")));
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Wait for the page to reach a specific ready state.
    pub async fn wait_for_load_state(&self, timeout_ms: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let state = self.evaluate_as_string("document.readyState").await?;
            if state == "complete" {
                return Ok(());
            }
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout("wait_for_load_state".into()));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

async fn enable_network(page: &Page) -> Result<()> {
    page.cmd("Network.enable", json!({})).await.map_err(|e| {
        WispError::Browser(BrowserError::CdpConnection(format!(
            "Network.enable failed: {e}"
        )))
    })?;
    Ok(())
}

fn is_load_event(event: &CdpEvent) -> bool {
    event.method == "Page.loadEventFired"
        || (event.method == "Page.lifecycleEvent"
            && event.params.get("name").and_then(|n| n.as_str()) == Some("load"))
}

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
async fn recv_navigation_status(
    rx: &mut tokio::sync::broadcast::Receiver<CdpEvent>,
    sid: &str,
) -> Result<u16> {
    use tokio::sync::broadcast::error::RecvError;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
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

/// 导航到 URL 并等待页面加载完成，返回导航状态码。
pub async fn do_goto(page: &mut Page, url: &str) -> Result<u16> {
    enable_network(page).await?;
    let mut event_rx = page.session.subscribe_events();
    let sid = page.session_id.clone();
    page.cmd("Page.navigate", json!({ "url": url })).await?;
    // Wait for page load using lifecycle event or timeout
    wait_for_load(page).await?;
    // 导航后刷新 frame_id，避免跨域导航后 isolated world 创建失败
    page.refresh_frame_id().await;
    recv_navigation_status(&mut event_rx, &sid).await
}

/// 重新加载页面并等待加载完成。
pub async fn do_reload(page: &Page) -> Result<()> {
    page.cmd("Page.reload", json!({})).await?;
    wait_for_load(page).await
}

async fn wait_for_load(page: &Page) -> Result<()> {
    let sid = page.session_id.clone();
    let mut rx = page.session.subscribe_events();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(15000);
    let start = std::time::Instant::now();
    let mut found = false;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(event)) => {
                let match_session =
                    event.session_id.as_deref() == Some(sid.as_str()) || event.session_id.is_none();
                if match_session && is_load_event(&event) {
                    found = true;
                    break;
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    tracing::debug!(
        "wait_for_load: 耗时 {}ms, 结果={}",
        start.elapsed().as_millis(),
        if found {
            "Ok(找到新事件)"
        } else {
            "超时(15s)"
        }
    );
    if !found {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn make_event(method: &str, params: serde_json::Value, session_id: Option<&str>) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: session_id.map(std::string::ToString::to_string),
        }
    }

    #[tokio::test]
    async fn recv_navigation_status_returns_status_code() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        tx.send(make_event(
            "Network.responseReceived",
            serde_json::json!({ "type": "XHR", "response": { "status": 204 } }),
            Some("sid"),
        ))
        .unwrap();
        tx.send(make_event(
            "Network.responseReceived",
            serde_json::json!({ "type": "Document", "response": { "status": 200 } }),
            Some("sid"),
        ))
        .unwrap();

        let status = recv_navigation_status(&mut rx, "sid")
            .await
            .expect("应返回状态码");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn recv_navigation_status_loading_failed_returns_error() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        tx.send(make_event(
            "Network.loadingFailed",
            serde_json::json!({ "type": "Document", "errorText": "net::ERR_PROXY_CONNECTION_FAILED" }),
            Some("sid"),
        ))
        .unwrap();

        let result = recv_navigation_status(&mut rx, "sid").await;
        assert!(result.is_err(), "loadingFailed 应返回错误");
        let err = result.unwrap_err();
        match err {
            WispError::Browser(BrowserError::CdpConnection(msg)) => {
                assert!(msg.contains("net::ERR_PROXY_CONNECTION_FAILED"));
            }
            _ => panic!("应是 CdpConnection 错误，实际: {err:?}"),
        }
    }

    #[tokio::test]
    async fn recv_navigation_status_timeout_returns_200() {
        let (_tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        let status = recv_navigation_status(&mut rx, "sid")
            .await
            .expect("超时应返回默认 200");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn recv_navigation_status_ignores_other_session() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        tx.send(make_event(
            "Network.responseReceived",
            serde_json::json!({ "type": "Document", "response": { "status": 404 } }),
            Some("other-sid"),
        ))
        .unwrap();
        tx.send(make_event(
            "Network.responseReceived",
            serde_json::json!({ "type": "Document", "response": { "status": 200 } }),
            Some("sid"),
        ))
        .unwrap();

        let status = recv_navigation_status(&mut rx, "sid")
            .await
            .expect("应返回状态码");
        assert_eq!(status, 200);
    }
}
