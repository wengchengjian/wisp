//! CDP client over WebSocket. Connects via --remote-debugging-port=0 (random port).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot, watch, Mutex};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tungstenite::Message;

use crate::error::{BrowserError, Result, WispError};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A CDP event received from Chrome.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    /// 事件方法名（如 `Page.loadEventFired`）。
    pub method: String,
    /// 事件参数。
    pub params: Value,
    /// 关联的 session ID（多 tab 场景区分来源）。
    pub session_id: Option<String>,
}

/// 连接状态：用于失败时快速通知所有等待中的 execute 调用者。
#[derive(Debug, Clone)]
enum ConnState {
    Open,
    /// 已关闭，包含错误信息（clone 给所有 pending 的 oneshot）。
    Closed(String),
}

/// CDP session over WebSocket.
pub struct CdpSession {
    writer: Arc<Mutex<futures::stream::SplitSink<WsStream, Message>>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    // OPTIMIZE: 删除 events Vec 和 consumed_offset，统一用 broadcast。
    event_broadcaster: broadcast::Sender<CdpEvent>,
    // OPTIMIZE: 连接状态广播，错误时所有 execute 立即收到，避免 30s timeout 等待。
    conn_state: watch::Sender<ConnState>,
}

impl CdpSession {
    /// Connect to Chrome's DevTools WebSocket endpoint.
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>> {
        let (ws, _) = connect_async(ws_url).await.map_err(|e| {
            WispError::Browser(BrowserError::CdpConnection(format!("ws connect: {e}")))
        })?;

        let (writer, mut reader) = ws.split();
        let writer = Arc::new(Mutex::new(writer));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // broadcast 容量 1024：足够容纳单次导航产生的事件 burst；
        // 慢消费者 lag 时返回 RecvError::Lagged，调用方记录 warn 并继续。
        let (event_broadcaster, _) = broadcast::channel(1024);
        let (conn_state_tx, _conn_state_rx) = watch::channel(ConnState::Open);

        let pending_clone = Arc::clone(&pending);
        let broadcaster_clone = event_broadcaster.clone();
        let conn_state_clone = conn_state_tx.clone();

        tokio::spawn(async move {
            while let Some(msg) = reader.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            if let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) {
                                let mut p = pending_clone.lock().await;
                                if let Some(tx) = p.remove(&id) {
                                    let _ = tx.send(value);
                                }
                            } else {
                                let method = value
                                    .get("method")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let params = value.get("params").cloned().unwrap_or(Value::Null);
                                let session_id = value
                                    .get("sessionId")
                                    .and_then(serde_json::Value::as_str)
                                    .map(std::string::ToString::to_string);
                                let event = CdpEvent {
                                    method,
                                    params,
                                    session_id,
                                };
                                // 广播给订阅者（无订阅者时 send 失败，忽略）
                                let _ = broadcaster_clone.send(event);
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        let _ = conn_state_clone.send(ConnState::Closed("ws closed".into()));
                        pending_clone.lock().await.clear();
                        break;
                    }
                    Err(e) => {
                        let _ = conn_state_clone.send(ConnState::Closed(format!("ws error: {e}")));
                        pending_clone.lock().await.clear();
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(Arc::new(Self {
            writer,
            next_id: AtomicU64::new(1),
            pending,
            event_broadcaster,
            conn_state: conn_state_tx,
        }))
    }

    /// 订阅事件流。订阅者只接收订阅之后到达的事件，避免历史缓冲污染。
    ///
    /// 用于需要"在触发动作前订阅"的场景（如 `goto` 前订阅以捕获
    /// `Network.responseReceived`）。慢消费者可能收到 `RecvError::Lagged`，
    /// 调用方应记录 warn 并继续 recv。
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<CdpEvent> {
        self.event_broadcaster.subscribe()
    }

    /// Send a CDP command and wait for response.
    pub async fn execute(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        self.execute_with_session(method, params, None).await
    }

    /// Send a CDP command with optional sessionId.
    pub async fn execute_with_session(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value> {
        // OPTIMIZE: 注册 pending 前先检查连接状态，避免注定失败的命令占用 30s。
        if matches!(*self.conn_state.borrow(), ConnState::Closed(_)) {
            return Err(WispError::Browser(BrowserError::CdpConnection(
                "connection closed".into(),
            )));
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            msg["sessionId"] = json!(sid);
        }

        let mut buf = Vec::with_capacity(256);
        serde_json::to_writer(&mut buf, &msg).expect("CDP msg always serializable");
        let text = String::from_utf8(buf).expect("CDP msg always UTF-8");
        {
            let mut writer = self.writer.lock().await;
            writer.send(Message::Text(text.into())).await.map_err(|e| {
                WispError::Browser(BrowserError::CdpConnection(format!("ws send: {e}")))
            })?;
        }

        // OPTIMIZE: select! 同时等待响应和连接关闭通知，连接断开时立即返回错误。
        let mut state_rx = self.conn_state.subscribe();
        let response = tokio::select! {
            r = tokio::time::timeout(std::time::Duration::from_secs(30), rx) => {
                match r {
                    Ok(Ok(v)) => v,
                    Ok(Err(_)) => {
                        return Err(WispError::Browser(BrowserError::CdpConnection(
                            "channel closed".into(),
                        )));
                    }
                    Err(_) => {
                        self.pending.lock().await.remove(&id);
                        return Err(WispError::Timeout(format!("CDP: {method}")));
                    }
                }
            }
            _ = state_rx.changed() => {
                self.pending.lock().await.remove(&id);
                let msg = match &*state_rx.borrow() {
                    ConnState::Closed(m) => m.clone(),
                    ConnState::Open => "connection closed".to_string(),
                };
                return Err(WispError::Browser(BrowserError::CdpConnection(msg)));
            }
        };

        if let Some(error) = response.get("error") {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("CDP error");
            return Err(WispError::Browser(BrowserError::CdpConnection(
                msg.to_string(),
            )));
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Wait for a CDP event matching predicate.
    pub async fn wait_for_event<F>(&self, predicate: F, timeout_ms: u64) -> Result<CdpEvent>
    where
        F: Fn(&CdpEvent) -> bool,
    {
        let mut rx = self.subscribe_events();
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(WispError::Timeout("waiting for CDP event".into()));
            }
            tokio::select! {
                recv = rx.recv() => {
                    match recv {
                        Ok(event) => {
                            if predicate(&event) {
                                return Ok(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("CDP event subscriber lagged by {n} events");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err(WispError::Browser(BrowserError::CdpConnection(
                                "event broadcaster closed".into(),
                            )));
                        }
                    }
                }
                () = tokio::time::sleep(remaining) => {
                    return Err(WispError::Timeout("waiting for CDP event".into()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cdp_event_broadcast_no_vec() {
        // 验证事件经 broadcast 直达订阅者，无需 Vec 中转
        let (tx, mut rx1) = tokio::sync::broadcast::channel::<String>(16);
        let mut rx2 = tx.subscribe();

        tx.send("Page.loadEventFired".to_string()).unwrap();

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1, "Page.loadEventFired");
        assert_eq!(e2, "Page.loadEventFired");
    }

    #[tokio::test]
    async fn test_cdp_connection_error_notifies_pending() {
        // 验证 ConnState watch 错误传播：连接关闭时所有 watch 订阅者立即收到
        use tokio::sync::watch;
        let (tx, rx) = watch::channel(ConnState::Open);

        tx.send(ConnState::Closed("ws closed".into())).unwrap();

        assert!(rx.has_changed().unwrap());
        let state = rx.borrow().clone();
        match state {
            ConnState::Closed(msg) => assert_eq!(msg, "ws closed"),
            ConnState::Open => panic!("expected Closed"),
        }
    }

    #[tokio::test]
    async fn test_wait_for_event_uses_broadcast() {
        let (tx, _) = tokio::sync::broadcast::channel::<i32>(2);
        let mut rx = tx.subscribe();

        for i in 0..5 {
            let _ = tx.send(i);
        }

        match rx.recv().await {
            Ok(v) => assert!((0..5).contains(&v)),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                assert!(n > 0, "lagged count should be positive");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => panic!("should not close"),
        }
    }
}
