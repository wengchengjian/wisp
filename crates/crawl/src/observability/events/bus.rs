//! 事件总线：管理监听器并分发事件。

use crate::CrawlEvent;
use futures::Stream;
use parking_lot::RwLock;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{EventCallback, EventListener};

/// 事件总线：单生产者（Engine）、多消费者（listener / subscription）。
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<EventBusInner>,
}

struct EventBusInner {
    listeners: RwLock<Vec<(u64, EventListener)>>,
    next_id: AtomicU64,
}

/// 订阅句柄：持有事件流，Drop 时自动注销监听器。
#[must_use]
pub struct Subscription {
    bus: EventBus,
    id: u64,
    sender: Option<mpsc::Sender<CrawlEvent>>,
    inner: ReceiverStream<CrawlEvent>,
}

impl EventBus {
    /// 创建空事件总线。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(EventBusInner {
                listeners: RwLock::new(Vec::new()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// 注册事件监听器，返回可手动注销的 id。
    pub fn on(&self, listener: impl EventCallback + Send + Sync + 'static) -> u64 {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let boxed: EventListener = Arc::new(move |event| listener.call(event));
        self.inner.listeners.write().push((id, boxed));
        id
    }

    /// 订阅事件流：所有订阅者都会收到同一份事件。
    pub fn subscribe(&self, capacity: usize) -> Subscription {
        let (tx, rx) = mpsc::channel(capacity);
        let listener_tx = tx.clone();
        let id = self.on(move |event: CrawlEvent| {
            let tx = listener_tx.clone();
            Box::pin(async move {
                let _ = tx.send(event).await;
            })
        });
        Subscription {
            bus: self.clone(),
            id,
            sender: Some(tx),
            inner: ReceiverStream::new(rx),
        }
    }

    /// 注销指定 id 的监听器。
    pub fn unsubscribe(&self, id: u64) {
        self.inner.listeners.write().retain(|(i, _)| *i != id);
    }

    /// 发射事件（无 listener 时为 no-op）。
    pub async fn emit(&self, event: CrawlEvent) {
        let listeners = self.inner.listeners.read().clone();
        if listeners.is_empty() {
            return;
        }
        for (_, listener) in &listeners {
            listener(event.clone()).await;
        }
    }

    /// 是否有监听器。
    pub fn has_listeners(&self) -> bool {
        !self.inner.listeners.read().is_empty()
    }

    /// 监听器数量。
    pub fn listener_count(&self) -> usize {
        self.inner.listeners.read().len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Subscription {
    pub(crate) fn sender(&self) -> mpsc::Sender<CrawlEvent> {
        self.sender.as_ref().expect("closed subscription").clone()
    }

    /// 注销监听器并关闭本订阅的发送端，使流在缓冲事件耗尽后自然结束。
    pub(crate) fn close(&mut self) {
        self.sender.take();
        self.bus.unsubscribe(self.id);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.close();
    }
}

impl Stream for Subscription {
    type Item = CrawlEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}
