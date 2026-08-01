//! CDP 事件解析、缓冲与后台读取。

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};
use tungstenite::Message;

use super::WsStream;

/// 历史事件缓冲上限。`events` Vec 仅用于 `wait_for_event` 兼容路径，
/// 主要消费者是 broadcast 订阅者；设置上限防止长爬取中内存无限增长。
const MAX_BUFFERED_EVENTS: usize = 1024;

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

async fn handle_ws_message(
    msg: std::result::Result<Message, tungstenite::Error>,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    events: &Arc<Mutex<Vec<CdpEvent>>>,
    notify: &Arc<tokio::sync::Notify>,
    broadcaster: &tokio::sync::broadcast::Sender<CdpEvent>,
) -> bool {
    match msg {
        Ok(Message::Text(text)) => {
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                if let Some(id) = value.get("id").and_then(|i| i.as_u64()) {
                    let mut p = pending.lock().await;
                    if let Some(tx) = p.remove(&id) {
                        let _ = tx.send(value);
                    }
                } else {
                    let event = event_from_value(&value);
                    let _ = broadcaster.send(event.clone());
                    let mut evts = events.lock().await;
                    evts.push(event);
                    if evts.len() > MAX_BUFFERED_EVENTS {
                        let excess = evts.len() - MAX_BUFFERED_EVENTS;
                        evts.drain(..excess);
                    }
                    drop(evts);
                    notify.notify_waiters();
                }
            }
            true
        }
        Ok(Message::Close(_)) => false,
        Err(_) => false,
        _ => true,
    }
}

fn event_from_value(value: &Value) -> CdpEvent {
    CdpEvent {
        method: value
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string(),
        params: value.get("params").cloned().unwrap_or(Value::Null),
        session_id: value
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
    }
}

pub(super) fn spawn_event_reader(
    mut reader: futures::stream::SplitStream<WsStream>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    events: Arc<Mutex<Vec<CdpEvent>>>,
    notify: Arc<tokio::sync::Notify>,
    broadcaster: tokio::sync::broadcast::Sender<CdpEvent>,
) {
    tokio::spawn(async move {
        while let Some(msg) = reader.next().await {
            if !handle_ws_message(msg, &pending, &events, &notify, &broadcaster).await {
                break;
            }
        }
    });
}
