//! 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
//!
//! ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
//! 新策略（如 Playwright）可实现此 trait 零侵入注入。

use std::time::Duration;

use async_trait::async_trait;

use wisp_browser::cdp::CdpEvent;
use wisp_browser::Page;
use wisp_core::error::{BrowserError, Result, WispError};

use wisp_core::{Request, Response};

/// 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
///
/// ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
/// 新策略（如 Playwright）可实现此 trait 零侵入注入。
///
/// 调用方（`FetchClient::fetch_browser`）保证：
/// - 调用前已 `acquire` page
/// - 调用后由调用方 `close` page
/// - 120s 总超时由调用方包装
#[async_trait]
pub trait BrowserFetchStrategy: Send + Sync {
    /// 执行浏览器导航 + 后处理（CF 挑战 / 人类行为 / 等待选择器等）。
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response>;
}

/// 从事件流中接收 `Network.responseReceived` (type=Document) 事件并提取状态码。
///
/// 必须在 `goto` 之前订阅 `event_rx`，否则可能丢失事件。
/// 5s 超时：导航通常在 1-3s 内完成，5s 足够覆盖慢速页面。
///
/// 特殊处理：若先收到 `Network.loadingFailed` (type=Document)，说明导航请求失败
///（如代理连接失败、DNS 解析失败），立即返回错误，不空等 5s 超时。
#[allow(dead_code)] // PR2 后续 task 将由 FetchClient 接入
pub(crate) async fn recv_navigation_status(
    rx: &mut tokio::sync::broadcast::Receiver<CdpEvent>,
    sid: &str,
) -> Result<u16> {
    use tokio::sync::broadcast::error::RecvError;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(event)) => {
                let match_session =
                    event.session_id.as_deref() == Some(sid) || event.session_id.is_none();
                if !match_session {
                    continue;
                }

                // 导航请求失败（代理/DNS/网络问题）：立即返回错误
                if event.method == "Network.loadingFailed" {
                    let is_doc =
                        event.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                    if is_doc {
                        let error_text = event
                            .params
                            .get("errorText")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown");
                        tracing::warn!(
                            "recv_navigation_status: Network.loadingFailed errorText={error_text}"
                        );
                        return Err(WispError::Browser(BrowserError::CdpConnection(format!(
                            "navigation loading failed: {error_text}"
                        ))));
                    }
                    continue;
                }

                if event.method != "Network.responseReceived" {
                    continue;
                }
                let is_doc =
                    event.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                if !is_doc {
                    continue;
                }
                return event
                    .params
                    .get("response")
                    .and_then(|r| r.get("status"))
                    .and_then(serde_json::Value::as_u64)
                    .map(|s| s as u16)
                    .ok_or_else(|| {
                        WispError::Browser(BrowserError::CdpConnection(
                            "Network.responseReceived missing response.status".into(),
                        ))
                    });
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
                // 超时不返回错误：CF 挑战页面可能不触发 Network.responseReceived (type=Document)
                // 事件（CF 用 JavaScript 挑战，非标准 HTTP 响应流程）。
                // 返回默认 200，让流程继续到 CF 挑战解决阶段。
                tracing::warn!(
                    "capture_navigation_status: 5s 内未收到 Network.responseReceived，\
                     返回默认 200（CF 挑战页面可能不触发此事件）"
                );
                return Ok(200);
            }
        }
    }
}

/// 从浏览器页面提取统一 Response。
///
/// ARCH: 从 FetchClient::extract_browser_response 提取为公共 helper，
/// 供 DynamicStrategy / StealthStrategy 复用。
#[allow(dead_code)] // PR2 后续 task 将由 FetchClient 接入
pub(crate) async fn extract_browser_response(
    page: &Page,
    req: &Request,
    nav_status: u16,
) -> Result<Response> {
    let html = page
        .evaluate_as_string("document.documentElement.outerHTML")
        .await?;
    let title = page.evaluate_as_string("document.title").await?;
    let final_url = page.evaluate_as_string("window.location.href").await?;

    let cookies_raw = page.evaluate_as_string("document.cookie").await?;
    let cookies: Vec<String> = cookies_raw
        .split(';')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();

    Ok(Response::from_browser(
        nav_status,
        final_url,
        html,
        title,
        cookies,
        req.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::broadcast;

    /// 构造一个 CdpEvent。
    fn make_event(method: &str, params: serde_json::Value, session_id: Option<&str>) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: session_id.map(std::string::ToString::to_string),
        }
    }

    #[tokio::test]
    async fn test_recv_navigation_status_returns_status_code() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        // 发送一个无关事件 + 一个 Document 响应事件
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "XHR", "response": {"status": 204}}),
            Some("sid"),
        ))
        .unwrap();
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "Document", "response": {"status": 200}}),
            Some("sid"),
        ))
        .unwrap();

        let status = recv_navigation_status(&mut rx, "sid").await.expect("应返回状态码");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn test_recv_navigation_status_loading_failed_returns_error() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        tx.send(make_event(
            "Network.loadingFailed",
            json!({"type": "Document", "errorText": "net::ERR_PROXY_CONNECTION_FAILED"}),
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
    async fn test_recv_navigation_status_timeout_returns_200() {
        let (_tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        // 不发送任何事件，等待超时
        let status = recv_navigation_status(&mut rx, "sid")
            .await
            .expect("超时应返回默认 200");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn test_recv_navigation_status_ignores_other_session() {
        let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
        // 不同 session 的事件应被忽略
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "Document", "response": {"status": 404}}),
            Some("other-sid"),
        ))
        .unwrap();
        // 匹配 session 的事件应被采用
        tx.send(make_event(
            "Network.responseReceived",
            json!({"type": "Document", "response": {"status": 200}}),
            Some("sid"),
        ))
        .unwrap();

        let status = recv_navigation_status(&mut rx, "sid").await.expect("应返回状态码");
        assert_eq!(status, 200);
    }

    /// MockStrategy：用于验证 trait 可实现、可调用。
    struct MockStrategy;

    #[async_trait]
    impl BrowserFetchStrategy for MockStrategy {
        async fn fetch(&self, _page: &mut Page, req: &Request) -> Result<Response> {
            Ok(Response::from_browser(
                200,
                req.url.clone(),
                "<html></html>".to_string(),
                "mock".to_string(),
                Vec::new(),
                req.clone(),
            ))
        }
    }

    #[test]
    fn test_trait_object_can_be_constructed() {
        let strategy: Box<dyn BrowserFetchStrategy> = Box::new(MockStrategy);
        // 仅验证 trait object 可构造（无 UB）
        let _ = &*strategy as *const dyn BrowserFetchStrategy;
    }
}
