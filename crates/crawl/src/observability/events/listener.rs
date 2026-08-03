//! 事件监听器类型与内置实现。

use crate::CrawlEvent;
use futures::future::BoxFuture;
use std::future::Future;
use std::sync::Arc;

use super::Metrics;

/// 事件监听器签名。
pub type EventListener = Arc<dyn Fn(CrawlEvent) -> BoxFuture<'static, ()> + Send + Sync>;

/// 可注册到 [`EventBus`] 的监听器。
///
/// `EventListener` 与 `Fn(CrawlEvent) -> Future` 闭包都实现该 trait；
/// 无捕获的 `async |event| {}` 可以直接传给 `EventBus::on`。
pub trait EventCallback {
    /// 处理单个事件。
    fn call(&self, event: CrawlEvent) -> BoxFuture<'static, ()>;
}

impl<F, Fut> EventCallback for F
where
    F: Fn(CrawlEvent) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn call(&self, event: CrawlEvent) -> BoxFuture<'static, ()> {
        Box::pin(self(event))
    }
}

impl<F, Fut> EventCallback for Arc<F>
where
    F: Fn(CrawlEvent) -> Fut + Send + Sync + ?Sized,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn call(&self, event: CrawlEvent) -> BoxFuture<'static, ()> {
        Box::pin(self.as_ref()(event))
    }
}

/// 便捷构造：日志监听器（tracing 输出）。
pub fn logging_listener() -> EventListener {
    Arc::new(|event: CrawlEvent| {
        Box::pin(async move {
            match &event {
                CrawlEvent::CrawlStarted { spider, start_urls } => {
                    tracing::info!("Crawl started: {} ({} URLs)", spider, start_urls);
                }
                CrawlEvent::CrawlFinished { stats } => {
                    tracing::info!("Crawl finished: {}", stats.summary());
                }
                CrawlEvent::Error {
                    url,
                    error,
                    attempt,
                } => {
                    tracing::warn!("Error (attempt {}): {} - {}", attempt, url, error);
                }
                CrawlEvent::BlockedDetected { url, status } => {
                    tracing::warn!("Blocked ({}): {}", status, url);
                }
                CrawlEvent::AutoUpgraded { url, from, to } => {
                    tracing::info!("Auto upgrade {:?} -> {:?}: {}", from, to, url);
                }
                _ => {}
            }
        })
    })
}

/// 便捷构造：指标收集监听器。
pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener {
    Arc::new(move |event: CrawlEvent| {
        let metrics = Arc::clone(&metrics);
        Box::pin(async move {
            match event {
                CrawlEvent::ResponseReceived {
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
                CrawlEvent::Item(_) => {
                    metrics
                        .items
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                CrawlEvent::Error { .. } => {
                    metrics
                        .errors
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                _ => {}
            }
        })
    })
}
