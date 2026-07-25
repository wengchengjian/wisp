//! 事件驱动生命周期 — 细粒度内部事件总线。
//!
//! 借鉴 Crawlee EventManager + Scrapy Signals 设计：
//! 关键路径（fetch 完成、item 产出、错误、Auto 升级）emit 事件，
//! 用户可注册监听器实现监控、日志、指标采集、告警。
//!
//! # 零成本原则
//!
//! 无 listener 时 emit 为 no-op（仅检查 Vec 是否为空）。
//!
//! # 与 CrawlEvent 关系
//!
//! `CrawlEvent` 保留作为 `run_stream` 的外部接口。
//! `EngineEvent` 是更细粒度的内部事件总线。
//! 可通过一个 listener 将 EngineEvent 桥接到 CrawlEvent channel。

use std::sync::Arc;
use futures::future::BoxFuture;

use crate::crawl::CrawlStats;
use crate::fetcher::FetchMode;

/// 引擎内部事件。
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// 爬取启动
    CrawlStarted {
        /// Spider 名称。
        spider: String,
        /// 起始 URL 数量。
        start_urls: usize,
    },
    /// 爬取完成
    CrawlFinished {
        /// 统计信息。
        stats: CrawlStats,
    },
    /// 请求被调度
    RequestScheduled {
        /// 请求 URL。
        url: String,
        /// 深度。
        depth: u32,
    },
    /// 响应接收
    ResponseReceived {
        /// 请求 URL。
        url: String,
        /// 状态码。
        status: u16,
        /// 耗时（毫秒）。
        elapsed_ms: u64,
        /// 是否来自缓存。
        from_cache: bool,
    },
    /// Item 产出
    ItemScraped {
        /// 来源 URL。
        url: String,
    },
    /// 错误发生
    ErrorOccurred {
        /// 请求 URL。
        url: String,
        /// 错误信息。
        error: String,
        /// 重试次数。
        attempt: u32,
    },
    /// 检测到封锁
    BlockedDetected {
        /// 请求 URL。
        url: String,
        /// 状态码。
        status: u16,
    },
    /// Auto 模式升级
    AutoUpgraded {
        /// 请求 URL。
        url: String,
        /// 原模式。
        from: FetchMode,
        /// 新模式。
        to: FetchMode,
    },
    /// 并发数变更
    ConcurrencyChanged {
        /// 原并发数。
        old: usize,
        /// 新并发数。
        new: usize,
    },
    /// Checkpoint 保存
    CheckpointSaved {
        /// 待处理请求数。
        pending: usize,
    },
}

/// 事件监听器签名。
pub type EventListener = Arc<dyn Fn(EngineEvent) -> BoxFuture<'static, ()> + Send + Sync>;

/// 事件总线：管理监听器并分发事件。
pub struct EventBus {
    listeners: Vec<EventListener>,
}

impl EventBus {
    /// 创建空事件总线。
    pub fn new() -> Self {
        Self { listeners: Vec::new() }
    }

    /// 注册事件监听器。
    pub fn on(&mut self, listener: EventListener) {
        self.listeners.push(listener);
    }

    /// 发射事件（无 listener 时为 no-op）。
    pub async fn emit(&self, event: EngineEvent) {
        if self.listeners.is_empty() {
            return;
        }
        for listener in &self.listeners {
            listener(event.clone()).await;
        }
    }

    /// 是否有监听器。
    pub fn has_listeners(&self) -> bool {
        !self.listeners.is_empty()
    }

    /// 监听器数量。
    pub fn listener_count(&self) -> usize {
        self.listeners.len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
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
                EngineEvent::ErrorOccurred { url, error, attempt } => {
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
                EngineEvent::ResponseReceived { elapsed_ms, from_cache, .. } => {
                    metrics.responses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if from_cache {
                        metrics.cache_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    metrics.total_elapsed_ms.fetch_add(elapsed_ms, std::sync::atomic::Ordering::Relaxed);
                }
                EngineEvent::ItemScraped { .. } => {
                    metrics.items.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                EngineEvent::ErrorOccurred { .. } => {
                    metrics.errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                _ => {}
            }
        })
    })
}

/// 简单指标收集器。
#[derive(Debug, Default)]
pub struct Metrics {
    /// 响应数。
    pub responses: std::sync::atomic::AtomicUsize,
    /// Item 数。
    pub items: std::sync::atomic::AtomicUsize,
    /// 错误数。
    pub errors: std::sync::atomic::AtomicUsize,
    /// 缓存命中数。
    pub cache_hits: std::sync::atomic::AtomicUsize,
    /// 总耗时（毫秒）。
    pub total_elapsed_ms: std::sync::atomic::AtomicU64,
}

impl Metrics {
    /// 创建新的指标收集器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 平均响应时间（毫秒）。
    pub fn avg_response_ms(&self) -> u64 {
        let responses = self.responses.load(std::sync::atomic::Ordering::Relaxed);
        if responses == 0 { return 0; }
        self.total_elapsed_ms.load(std::sync::atomic::Ordering::Relaxed) / responses as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_event_bus_no_listeners() {
        let bus = EventBus::new();
        assert!(!bus.has_listeners());
        // emit should be no-op
        bus.emit(EngineEvent::CrawlStarted { spider: "test".into(), start_urls: 1 }).await;
    }

    #[tokio::test]
    async fn test_event_bus_with_listener() {
        let mut bus = EventBus::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        bus.on(Arc::new(move |_event: EngineEvent| {
            let c = Arc::clone(&counter_clone);
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        }));

        assert!(bus.has_listeners());
        assert_eq!(bus.listener_count(), 1);

        bus.emit(EngineEvent::CrawlStarted { spider: "test".into(), start_urls: 1 }).await;
        bus.emit(EngineEvent::ItemScraped { url: "http://x.com".into() }).await;

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_metrics_listener() {
        let metrics = Arc::new(Metrics::new());
        let mut bus = EventBus::new();
        bus.on(metrics_listener(Arc::clone(&metrics)));

        bus.emit(EngineEvent::ResponseReceived {
            url: "http://x.com".into(),
            status: 200,
            elapsed_ms: 150,
            from_cache: false,
        }).await;

        bus.emit(EngineEvent::ItemScraped { url: "http://x.com".into() }).await;

        assert_eq!(metrics.responses.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.items.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.avg_response_ms(), 150);
    }
}
