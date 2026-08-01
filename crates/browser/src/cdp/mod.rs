//! CDP client over WebSocket. Connects via --remote-debugging-port=0 (random port).

mod command;
mod connection;
mod event;
mod wait;

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tungstenite::Message;

pub use event::CdpEvent;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// CDP session over WebSocket.
pub struct CdpSession {
    writer: Arc<Mutex<futures::stream::SplitSink<WsStream, Message>>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    events: Arc<Mutex<Vec<CdpEvent>>>,
    /// 已消费事件偏移量（用于定期 drain 防止内存无限增长）。
    consumed_offset: Arc<Mutex<usize>>,
    event_notify: Arc<tokio::sync::Notify>,
    /// 事件广播：订阅者在注册后只接收新事件，避免历史缓冲污染。
    /// 用于需要"在触发动作前订阅"的场景（如捕获导航状态码）。
    event_broadcaster: tokio::sync::broadcast::Sender<CdpEvent>,
}

impl CdpSession {
    /// 订阅事件流。订阅者只接收订阅之后到达的事件，避免历史缓冲污染。
    ///
    /// 用于需要"在触发动作前订阅"的场景（如 `goto` 前订阅以捕获
    /// `Network.responseReceived`）。慢消费者可能收到 `RecvError::Lagged`，
    /// 调用方应记录 warn 并继续 recv。
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<CdpEvent> {
        self.event_broadcaster.subscribe()
    }
}
