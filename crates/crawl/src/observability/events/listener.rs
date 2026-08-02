//! 事件监听器类型与内置实现。

use std::future::Future;
use std::sync::Arc;

use futures::future::BoxFuture;

use super::{EngineEvent, Metrics};

/// 事件监听器签名。
pub type EventListener = Arc<dyn Fn(EngineEvent) -> BoxFuture<'static, ()> + Send + Sync>;

/// 可注册到 [`EventBus`] 的监听器。
///
/// `EventListener` 与 `Fn(EngineEvent) -> Future` 闭包都实现该 trait；
/// 无捕获的 `async |event| {}` 可以直接传给 `EventBus::on`。
pub trait EventCallback {
    /// 处理单个事件。
    fn call(&self, event: EngineEvent) -> BoxFuture<'static, ()>;
}

impl<F, Fut> EventCallback for F
where
    F: Fn(EngineEvent) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn call(&self, event: EngineEvent) -> BoxFuture<'static, ()> {
        Box::pin(self(event))
    }
}

impl<F, Fut> EventCallback for Arc<F>
where
    F: Fn(EngineEvent) -> Fut + Send + Sync + ?Sized,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn call(&self, event: EngineEvent) -> BoxFuture<'static, ()> {
        Box::pin(self.as_ref()(event))
    }
}

/// 便捷构造：日志监听器（tracing 输出）。
pub fn logging_listener() -> EventListener {
    Arc::new(|event: EngineEvent| {
        Box::pin(async move {
            match &event {
                EngineEvent::CrawlStarted { spider, start_urls } => {
                    tracing::info!("Crawl started: {} ({} URLs)", spider, start_urls);
                }
                EngineEvent::CrawlFinished { stats } => {
                    tracing::info!("Crawl finished: {}", stats.summary());
                }
                EngineEvent::ErrorOccurred {
                    url,
                    error,
                    attempt,
                } => {
                    tracing::warn!("Error (attempt {}): {} - {}", attempt, url, error);
                }
                EngineEvent::BlockedDetected { url, status } => {
                    tracing::warn!("Blocked ({}): {}", status, url);
                }
                EngineEvent::AutoUpgraded { url, from, to } => {
                    tracing::info!("Auto upgrade {:?} -> {:?}: {}", from, to, url);
                }
                _ => {}
            }
        })
    })
}

/// 便捷构造：指标收集监听器。
pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener {
    Arc::new(move |event: EngineEvent| {
        let metrics = Arc::clone(&metrics);
        Box::pin(async move {
            match event {
                EngineEvent::ResponseReceived {
                    elapsed_ms,
                    from_cache,
                    ..
                } => {
                    metrics
                        .responses
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if from_cache {
                        metrics
                            .cache_hits
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    metrics
                        .total_elapsed_ms
                        .fetch_add(elapsed_ms, std::sync::atomic::Ordering::Relaxed);
                }
                EngineEvent::ItemScraped { .. } => {
                    metrics
                        .items
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                EngineEvent::ErrorOccurred { .. } => {
                    metrics
                        .errors
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                _ => {}
            }
        })
    })
}
