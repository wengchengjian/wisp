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
/// Routes command responses by ID and broadcasts events.
pub struct CdpSession {
    transport: Arc<Mutex<PipeTransport>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    event_tx: broadcast::Sender<CdpEvent>,
}

impl CdpSession {
    pub fn new(transport: PipeTransport) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            transport: Arc::new(Mutex::new(transport)),
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
        })
    }

    /// Send a CDP command and wait for its response.
    /// NEVER call this with "Runtime.enable" or "Console.enable".
    pub async fn execute(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let msg = json!({ "id": id, "method": method, "params": params });
        self.transport.lock().await.send(&msg).await?;

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
        let transport = Arc::clone(&self.transport);
        let pending = Arc::clone(&self.pending);
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            loop {
                let msg = {
                    let mut t = transport.lock().await;
                    match t.recv().await {
                        Ok(m) => m,
                        Err(_) => break, // pipe closed
                    }
                };

                if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                    // Response to a command
                    let mut p = pending.lock().await;
                    if let Some(tx) = p.remove(&id) {
                        let _ = tx.send(msg);
                    }
                } else if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                    // Event
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
