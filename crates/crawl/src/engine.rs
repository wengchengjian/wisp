//! Engine 实现 - 从 mod.rs 拆分，降低圈复杂度。
//!
//! 核心拆解：
//! - `EngineContext` 打包单次 run 状态（替代 20+ 个 Arc 变量传递）
//! - `process_request()` 处理单个请求（替代 200 行嵌套闭包）
//! - `fetch_with_retry()` 抓取 + 同步重试循环（transport 级错误恢复编排）
//! - `auto_upgrade_check()` Auto 模式升级检查
//!
//! Task 3 重构：EngineContext 多 Spider 共享队列 + callback 路由，process_request
//! 调 `spider.handle()` 而非 `spider.parse()`，items 经 `CrawlEvent::Item` 事件交付。
//!
//! 架构收敛：`runner` 模块已并入本模块树，Engine/EngineBuilder/EngineConfig 是唯一
//! 公开接口，内部实现均为 crate 私有。

// 注：per-domain 信号量已删除。全局并发由 buffer_unordered(buffer_ceiling) 控制，
// 动态调整由 autoscale 负责。多域名公平性由用户通过 Request::priority 或 download_delay 管理。
use futures::stream::{self, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::Instrument;

use super::stats::SpiderStats;
use super::{
    CrawlEvent, CrawlRequest, CrawlState, CrawlStats, CrawlStream, Response, Spider, auto,
    middleware, scheduler,
};
use crate::Item;
use crate::control;
use crate::middleware::{ItemPipeline, Middleware, UaRotationMiddleware};
use crate::observability::events::EventBus;
use crate::runtime::autoscale::AutoscaledPool;
use wisp_core::error::Result;
use wisp_core::utils::sanitize_url;
use wisp_fetcher::{FetchClient, FetchMode};
use wisp_storage::Store;

// === 公开类型 ===
mod builder;
mod config;
mod lifecycle;
mod setup;
mod work;

// === 内部实现 ===
pub(crate) mod checkpoint;
pub(crate) mod context;
pub(crate) mod fetch;
pub(crate) mod guard;
pub(crate) mod request;
pub(crate) mod response;
pub(crate) mod router;

#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod tests;

pub use builder::EngineBuilder;
pub(crate) use checkpoint::maybe_persist_checkpoint;
pub use config::EngineConfig;
pub(crate) use context::{
    CfLockMap, EngineContext, EngineRunDraft, EngineState, QueueState, RunState, SpiderRegistry,
    build_crawl_context_for,
};
pub(crate) use fetch::fetch_with_retry;
pub(crate) use guard::{InFlightGuard, RunGuard};
pub(crate) use request::process_request;
pub(crate) use response::process_response;
pub(crate) use work::{build_final_stats, run_stream_driver, run_work_loop};

/// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
///
/// 配置与运行时资源分离：`config` 是唯一配置源，`runtime` 持有可共享资源。
#[derive(Clone)]
pub struct Engine {
    /// 唯一配置源。
    pub(crate) config: EngineConfig,
    /// 运行时资源。
    pub(crate) runtime: EngineRuntime,
    /// 运行时并发保护。
    pub(crate) running: Arc<AtomicBool>,
}

/// 运行时资源：可共享、可替换，但不属于用户配置。
#[derive(Clone)]
pub(crate) struct EngineRuntime {
    /// 共享 FetchClient（HTTP 连接池 + BrowserPool，跨 Spider 复用）。
    pub fetch_client: Arc<FetchClient>,
    /// Engine 控制状态（pause/resume/cancel/shutdown）。
    pub control: Arc<control::EngineControl>,
    /// 响应缓存存储（可选）。
    pub cache_store: Option<Arc<dyn Store>>,
    /// checkpoint 存储（可选）。
    pub checkpoint_store: Option<Arc<dyn Store>>,
    /// 上次 checkpoint 保存时间（用于时间间隔触发）。
    pub last_checkpoint_at: Arc<Mutex<Option<Instant>>>,
    /// checkpoint 保存进行中（防重入，避免并发全量快照重复）。
    pub checkpoint_saving: Arc<AtomicBool>,
    /// 自适应并发池（可选）。
    pub autoscale: Option<Arc<AutoscaledPool>>,
    /// 引擎事件总线。
    pub event_bus: Arc<EventBus>,
    /// UA 轮换中间件实例（可选）。
    pub ua_middleware: Option<Arc<UaRotationMiddleware>>,
    /// 用户自定义 Engine 级中间件。
    pub custom_middlewares: Vec<Arc<dyn Middleware>>,
    /// 共享 item pipeline。
    pub pipelines: Vec<Arc<dyn ItemPipeline>>,
}

/// Engine 运行时资源草稿：Builder 持有，build 时补上 control。
#[derive(Clone)]
pub(crate) struct EngineRuntimeDraft {
    /// 共享 FetchClient（HTTP 连接池 + BrowserPool，跨 Spider 复用）。
    pub fetch_client: Option<Arc<FetchClient>>,
    /// 响应缓存存储（可选）。
    pub cache_store: Option<Arc<dyn Store>>,
    /// checkpoint 存储（可选）。
    pub checkpoint_store: Option<Arc<dyn Store>>,
    /// 上次 checkpoint 保存时间（用于时间间隔触发）。
    pub last_checkpoint_at: Arc<Mutex<Option<Instant>>>,
    /// checkpoint 保存进行中（防重入，避免并发全量快照重复）。
    pub checkpoint_saving: Arc<AtomicBool>,
    /// 自适应并发池（可选）。
    pub autoscale: Option<Arc<AutoscaledPool>>,
    /// 引擎事件总线。
    pub event_bus: EventBus,
    /// UA 轮换中间件实例（可选）。
    pub ua_middleware: Option<Arc<UaRotationMiddleware>>,
    /// 用户自定义 Engine 级中间件。
    pub custom_middlewares: Vec<Arc<dyn Middleware>>,
    /// 共享 item pipeline。
    pub pipelines: Vec<Arc<dyn ItemPipeline>>,
}

impl EngineRuntimeDraft {
    /// 创建空资源草稿。
    pub(crate) fn new() -> Self {
        Self {
            fetch_client: None,
            cache_store: None,
            checkpoint_store: None,
            last_checkpoint_at: Arc::new(Mutex::new(None)),
            checkpoint_saving: Arc::new(AtomicBool::new(false)),
            autoscale: None,
            event_bus: EventBus::new(),
            ua_middleware: None,
            custom_middlewares: Vec::new(),
            pipelines: Vec::new(),
        }
    }

    /// 补上 control 后成为完整运行时。
    pub(crate) fn into_runtime(self, control: Arc<control::EngineControl>) -> EngineRuntime {
        EngineRuntime {
            fetch_client: self
                .fetch_client
                .expect("fetch client resolved before into_runtime"),
            control,
            cache_store: self.cache_store,
            checkpoint_store: self.checkpoint_store,
            last_checkpoint_at: self.last_checkpoint_at,
            checkpoint_saving: self.checkpoint_saving,
            autoscale: self.autoscale,
            event_bus: Arc::new(self.event_bus),
            ua_middleware: self.ua_middleware,
            custom_middlewares: self.custom_middlewares,
            pipelines: self.pipelines,
        }
    }
}

impl Default for EngineRuntimeDraft {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// 运行多个 Spider：共享队列 + callback 路由，每个 Spider 独立 until/stats。
    pub async fn run_many<S: Spider + 'static>(
        &self,
        spiders: Vec<S>,
    ) -> Result<(Vec<CrawlStats>, Vec<Item>)> {
        let (stream, outcome) = self.run_stream_many_inner(spiders);
        let mut items = Vec::new();
        let mut events = stream.events();
        while let Some(event) = events.next().await {
            if let CrawlEvent::Item(item) = event {
                items.push(item);
            }
        }
        let stats = outcome.await.map_err(|_| {
            wisp_core::error::WispError::Engine("run outcome channel closed".into())
        })??;
        Ok((stats, items))
    }

    /// 运行单个 Spider。返回 (统计, items)。
    pub async fn run<S: Spider + 'static>(&self, spider: S) -> Result<(CrawlStats, Vec<Item>)> {
        let (mut stats, items) = self.run_many(vec![spider]).await?;
        Ok((stats.remove(0), items))
    }

    /// 流式运行：边爬边产出事件（仅单 Spider 模式）。
    pub fn run_stream<S: Spider + 'static>(&self, spider: S) -> CrawlStream {
        let stream = self.run_stream_many(vec![spider]);
        CrawlStream {
            inner: Box::pin(stream.inner.map(|event| match event {
                CrawlEvent::DoneMany(mut stats) => {
                    CrawlEvent::Done(stats.pop().unwrap_or_default())
                }
                other => other,
            })),
        }
    }

    /// 流式运行多个 Spider：共享队列 + callback 路由，每个 Spider 独立 until/stats。
    pub fn run_stream_many<S: Spider + 'static>(&self, spiders: Vec<S>) -> CrawlStream {
        self.run_stream_many_inner(spiders).0
    }

    pub(crate) fn run_stream_many_inner<S: Spider + 'static>(
        &self,
        spiders: Vec<S>,
    ) -> (
        CrawlStream,
        tokio::sync::oneshot::Receiver<Result<Vec<CrawlStats>>>,
    ) {
        let subscription = self.runtime.event_bus.subscribe(128);
        let tx = subscription.sender();
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();
        let engine = self.clone();
        let spiders: Vec<Arc<dyn Spider>> = spiders
            .into_iter()
            .map(|s| Arc::new(s) as Arc<dyn Spider>)
            .collect();
        let driver = Box::pin(run_stream_driver(engine, spiders, tx, outcome_tx));
        let s = stream::unfold(
            (driver, subscription, false),
            |(mut driver, mut subscription, driver_done)| async move {
                if driver_done {
                    return subscription
                        .next()
                        .await
                        .map(|e| (e, (driver, subscription, true)));
                }
                tokio::select! {
                    biased;
                    event = subscription.next() => event.map(|e| (e, (driver, subscription, false))),
                    _ = &mut driver => {
                        subscription.close();
                        subscription.next().await.map(|e| (e, (driver, subscription, true)))
                    }
                }
            },
        );
        (CrawlStream { inner: Box::pin(s) }, outcome_rx)
    }

    /// 获取控制句柄（用于外部 pause/resume/cancel/shutdown）。
    pub fn control(&self) -> &Arc<control::EngineControl> {
        &self.runtime.control
    }

    /// 获取 Engine 配置。
    #[must_use]
    pub fn config(&self) -> EngineConfig {
        self.config.clone()
    }

    /// 关闭 Engine（停止所有运行中的爬取）。
    pub fn shutdown(&self) {
        self.runtime.control.shutdown();
    }
}
