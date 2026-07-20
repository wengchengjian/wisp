use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio::net::TcpStream;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, oneshot, Notify};
use std::collections::HashMap;
use tungstenite::Message;
use crate::error::{PatchrightError, Result};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Represents a Playwright protocol event (e.g., __create__, __dispose__)
#[derive(Debug, Clone)]
pub struct ProtocolEvent {
    pub guid: String,
    pub method: String,
    pub params: Value,
}

pub struct PlaywrightConnection {
    writer: Arc<Mutex<futures::stream::SplitSink<WsStream, Message>>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    /// Shared buffer for collected events
    events: Arc<Mutex<Vec<ProtocolEvent>>>,
    /// Notify when new events arrive
    event_notify: Arc<Notify>,
}

impl PlaywrightConnection {
    /// Connect to the Playwright driver WebSocket server
    pub async fn connect(url: &str) -> Result<(Self, tokio::task::JoinHandle<()>)> {
        let (ws, _) = connect_async(url).await
            .map_err(|e| PatchrightError::CdpError(format!("ws connect: {e}")))?;

        let (writer, reader) = ws.split();
        let writer = Arc::new(Mutex::new(writer));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));

        // Shared event buffer
        let events: Arc<Mutex<Vec<ProtocolEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let event_notify = Arc::new(Notify::new());
        let notify_clone = Arc::clone(&event_notify);

        // Spawn reader task
        let pending_clone = Arc::clone(&pending);
        let handle = tokio::spawn(async move {
            let mut reader = reader;
            while let Some(msg) = reader.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        tracing::debug!("RAW <<< {}", text);
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            // Check if this is a response (has "id" field)
                            if let Some(id) = value.get("id").and_then(|i| i.as_u64()) {
                                let mut p = pending_clone.lock().await;
                                if let Some(tx) = p.remove(&id) {
                                    let _ = tx.send(value);
                                }
                            } else {
                                // This is an event (no "id" field)
                                let guid = value.get("guid")
                                    .and_then(|g| g.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let method = value.get("method")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let params = value.get("params").cloned().unwrap_or(Value::Null);

                                let event = ProtocolEvent { guid, method, params };
                                tracing::debug!("EVENT: {} {} {:?}", event.guid, event.method, event.params);
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

        let conn = Self {
            writer,
            next_id: AtomicU64::new(1),
            pending,
            events,
            event_notify,
        };

        Ok((conn, handle))
    }

    /// Wait for a specific event matching the predicate, with timeout
    pub async fn wait_for_event<F>(&self, predicate: F, timeout_ms: u64) -> Result<ProtocolEvent>
    where
        F: Fn(&ProtocolEvent) -> bool,
    {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

        loop {
            // Check existing events first
            {
                let events = self.events.lock().await;
                if let Some(event) = events.iter().find(|e| predicate(e)) {
                    return Ok(event.clone());
                }
            }

            // Wait for new events or timeout
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(PatchrightError::Timeout("waiting for event".into()));
            }

            tokio::select! {
                _ = self.event_notify.notified() => {
                    // New event arrived, loop will check it
                }
                _ = tokio::time::sleep(remaining) => {
                    return Err(PatchrightError::Timeout("waiting for event".into()));
                }
            }
        }
    }

    /// Send a command and wait for response
    pub async fn send_command(&self, guid: &str, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let msg = json!({
            "id": id,
            "guid": guid,
            "method": method,
            "params": params,
            "metadata": {}
        });

        let text = serde_json::to_string(&msg).unwrap();
        tracing::debug!(">>> {}", text);
        self.writer.lock().await
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| PatchrightError::CdpError(format!("ws send: {e}")))?;

        let response = rx.await
            .map_err(|_| PatchrightError::CdpError("response channel closed".into()))?;

        tracing::debug!("<<< {}", serde_json::to_string(&response).unwrap_or_default());

        // Check for error in response
        if let Some(error) = response.get("error") {
            let msg = error.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str())
                .or_else(|| error.get("message").and_then(|m| m.as_str()))
                .unwrap_or("unknown error");
            return Err(PatchrightError::CdpError(msg.to_string()));
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }
}
