//! WebSocket 连接与 CdpSession 建立。

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::connect_async;

use super::event::spawn_event_reader;
use super::{CdpEvent, CdpSession, WsStream};
use wisp_core::error::{BrowserError, Result, WispError};

async fn open_ws(ws_url: &str) -> Result<WsStream> {
    let url = ws_url.to_string();
    let (ws, _) = tokio::task::spawn(async move { connect_async(&url).await })
        .await
        .map_err(|e| {
            WispError::Browser(BrowserError::CdpConnection(format!("ws connect task: {e}")))
        })?
        .map_err(|e| WispError::Browser(BrowserError::CdpConnection(format!("ws connect: {e}"))))?;
    Ok(ws)
}

impl CdpSession {
    /// Connect to Chrome's DevTools WebSocket endpoint.
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>> {
        let ws = open_ws(ws_url).await?;
        let (writer, reader) = ws.split();
        let writer = Arc::new(Mutex::new(writer));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let events: Arc<Mutex<Vec<CdpEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let event_notify = Arc::new(tokio::sync::Notify::new());
        let (event_broadcaster, _) = tokio::sync::broadcast::channel(1024);
        let consumed_offset = Arc::new(Mutex::new(0usize));
        spawn_event_reader(
            reader,
            Arc::clone(&pending),
            Arc::clone(&events),
            Arc::clone(&event_notify),
            event_broadcaster.clone(),
        );
        Ok(Arc::new(Self {
            writer,
            next_id: AtomicU64::new(1),
            pending,
            events,
            consumed_offset,
            event_notify,
            event_broadcaster,
        }))
    }
}
