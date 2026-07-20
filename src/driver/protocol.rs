use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio::net::TcpStream;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, oneshot};
use std::collections::HashMap;
use tungstenite::Message;
use crate::error::{PatchrightError, Result};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct PlaywrightConnection {
    writer: Arc<Mutex<futures::stream::SplitSink<WsStream, Message>>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
}

impl PlaywrightConnection {
    /// Connect to the Playwright driver WebSocket server
    pub async fn connect(url: &str) -> Result<(Self, tokio::task::JoinHandle<()>)> {
        let (ws, _) = connect_async(url).await
            .map_err(|e| PatchrightError::CdpError(format!("ws connect: {e}")))?;

        let (writer, reader) = ws.split();
        let writer = Arc::new(Mutex::new(writer));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));

        // Spawn reader task
        let pending_clone = Arc::clone(&pending);
        let handle = tokio::spawn(async move {
            let mut reader = reader;
            while let Some(msg) = reader.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            // Route response by id
                            if let Some(id) = value.get("id").and_then(|i| i.as_u64()) {
                                let mut p = pending_clone.lock().await;
                                if let Some(tx) = p.remove(&id) {
                                    let _ = tx.send(value);
                                }
                            }
                            // Events are ignored for now (can be added later)
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
        };

        Ok((conn, handle))
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
        self.writer.lock().await
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| PatchrightError::CdpError(format!("ws send: {e}")))?;

        let response = rx.await
            .map_err(|_| PatchrightError::CdpError("response channel closed".into()))?;

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
