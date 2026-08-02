use super::*;
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::broadcast;
use wisp_browser::Page;
use wisp_browser::cdp::CdpEvent;
use wisp_core::error::{BrowserError, Result, WispError};
use wisp_core::{Request, Response};

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
        json!({ "type": "XHR", "response": { "status": 204 } }),
        Some("sid"),
    ))
    .unwrap();
    tx.send(make_event(
        "Network.responseReceived",
        json!({ "type": "Document", "response": { "status": 200 } }),
        Some("sid"),
    ))
    .unwrap();

    let status = recv_navigation_status(&mut rx, "sid")
        .await
        .expect("应返回状态码");
    assert_eq!(status, 200);
}

#[tokio::test]
async fn test_recv_navigation_status_loading_failed_returns_error() {
    let (tx, mut rx) = broadcast::channel::<CdpEvent>(8);
    tx.send(make_event(
        "Network.loadingFailed",
        json!({ "type": "Document", "errorText": "net::ERR_PROXY_CONNECTION_FAILED" }),
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
        json!({ "type": "Document", "response": { "status": 404 } }),
        Some("other-sid"),
    ))
    .unwrap();
    // 匹配 session 的事件应被采用
    tx.send(make_event(
        "Network.responseReceived",
        json!({ "type": "Document", "response": { "status": 200 } }),
        Some("sid"),
    ))
    .unwrap();

    let status = recv_navigation_status(&mut rx, "sid")
        .await
        .expect("应返回状态码");
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
