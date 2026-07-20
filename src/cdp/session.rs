use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex};
use tokio::task::JoinHandle;
use serde_json::{json, Value};
use crate::cdp::pipe::PipeTransport;
use crate::error::{PatchrightError, Result};

/// A CDP event received from Chrome.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

/// Manages CDP communication over a pipe transport.
pub struct CdpSession {
    transport: Arc<PipeTransport>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    event_tx: broadcast::Sender<CdpEvent>,
    msg_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Value>>>,
}

impl CdpSession {
    pub fn new(transport: PipeTransport, msg_rx: tokio::sync::mpsc::UnboundedReceiver<Value>) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            transport: Arc::new(transport),
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            msg_rx: Arc::new(tokio::sync::Mutex::new(msg_rx)),
        })
    }

    /// Send a CDP command and wait for its response.
    pub async fn execute(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        self.execute_with_session(method, params, None).await
    }

    /// Send a CDP command with an optional sessionId (for target-specific commands).
    pub async fn execute_with_session(self: &Arc<Self>, method: &str, params: Value, session_id: Option<&str>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            msg["sessionId"] = json!(sid);
        }
        self.transport.send(&msg).await?;

        let response = rx.await
            .map_err(|_| PatchrightError::CdpError("response channel closed".into()))?;

        if let Some(error) = response.get("error") {
            let msg = error.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown CDP error");
            return Err(PatchrightError::CdpError(msg.to_string()));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Subscribe to CDP events.
    pub fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
    }

    /// Spawn background reader loop that routes responses and events.
    pub fn spawn_reader(self: &Arc<Self>) -> JoinHandle<()> {
        let pending = Arc::clone(&self.pending);
        let event_tx = self.event_tx.clone();
        let msg_rx = Arc::clone(&self.msg_rx);

        tokio::spawn(async move {
            loop {
                let msg = {
                    let mut rx = msg_rx.lock().await;
                    match rx.recv().await {
                        Some(m) => m,
                        None => break, // channel closed
                    }
                };

                if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                    let mut p = pending.lock().await;
                    if let Some(tx) = p.remove(&id) {
                        let _ = tx.send(msg);
                    }
                } else if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                    let event = CdpEvent {
                        method: method.to_string(),
                        params: msg.get("params").cloned().unwrap_or(Value::Null),
                        session_id: msg.get("sessionId").and_then(|s| s.as_str()).map(String::from),
                    };
                    let _ = event_tx.send(event);
                }
            }
        })
    }
}
