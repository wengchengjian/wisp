//! CDP client over WebSocket. Connects via --remote-debugging-port=0 (random port).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio::net::TcpStream;
use tungstenite::Message;

use crate::error::{WispError, Result};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A CDP event received from Chrome.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

/// CDP session over WebSocket.
pub struct CdpSession {
    writer: Arc<Mutex<futures::stream::SplitSink<WsStream, Message>>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    events: Arc<Mutex<Vec<CdpEvent>>>,
    event_notify: Arc<tokio::sync::Notify>,
}

impl CdpSession {
    /// Connect to Chrome's DevTools WebSocket endpoint.
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>> {
        let (ws, _) = connect_async(ws_url).await
            .map_err(|e| WispError::CdpError(format!("ws connect: {e}")))?;

        let (writer, mut reader) = ws.split();
        let writer = Arc::new(Mutex::new(writer));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));
        let events: Arc<Mutex<Vec<CdpEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let event_notify = Arc::new(tokio::sync::Notify::new());

        let pending_clone = Arc::clone(&pending);
        let events_clone = Arc::clone(&events);
        let notify_clone = Arc::clone(&event_notify);

        tokio::spawn(async move {
            while let Some(msg) = reader.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            if let Some(id) = value.get("id").and_then(|i| i.as_u64()) {
                                let mut p = pending_clone.lock().await;
                                if let Some(tx) = p.remove(&id) {
                                    let _ = tx.send(value);
                                }
                            } else {
                                let method = value.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
                                let params = value.get("params").cloned().unwrap_or(Value::Null);
                                let session_id = value.get("sessionId").and_then(|s| s.as_str()).map(|s| s.to_string());
                                let event = CdpEvent { method, params, session_id };
                                events_clone.lock().await.push(event);
                                notify_clone.notify_waiters();
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
        });

        Ok(Arc::new(Self { writer, next_id: AtomicU64::new(1), pending, events, event_notify }))
    }

    /// Send a CDP command and wait for response.
    pub async fn execute(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        self.execute_with_session(method, params, None).await
    }

    /// Send a CDP command with optional sessionId.
    pub async fn execute_with_session(self: &Arc<Self>, method: &str, params: Value, session_id: Option<&str>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            msg["sessionId"] = json!(sid);
        }

        let text = serde_json::to_string(&msg).unwrap();
        self.writer.lock().await.send(Message::Text(text.into())).await
            .map_err(|e| WispError::CdpError(format!("ws send: {e}")))?;

        let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await
            .map_err(|_| WispError::Timeout(format!("CDP: {method}")))?
            .map_err(|_| WispError::CdpError("channel closed".into()))?;

        if let Some(error) = response.get("error") {
            let msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("CDP error");
            return Err(WispError::CdpError(msg.to_string()));
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Wait for a CDP event matching predicate.
    pub async fn wait_for_event<F>(&self, predicate: F, timeout_ms: u64) -> Result<CdpEvent>
    where F: Fn(&CdpEvent) -> bool {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
        loop {
            {
                let events = self.events.lock().await;
                if let Some(event) = events.iter().find(|e| predicate(e)) {
                    return Ok(event.clone());
                }
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(WispError::Timeout("waiting for CDP event".into()));
            }
            tokio::select! {
                _ = self.event_notify.notified() => {}
                _ = tokio::time::sleep(remaining) => {
                    return Err(WispError::Timeout("waiting for CDP event".into()));
                }
            }
        }
    }
}
