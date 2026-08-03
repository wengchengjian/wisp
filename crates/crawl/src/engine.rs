//! Engine 实现 - 从 mod.rs 拆分，降低圈复杂度。
//!
//! 核心拆解：
//! - `EngineContext` 打包单次 run 状态（替代 20+ 个 Arc 变量传递）
//! - `process_request()` 处理单个请求（替代 200 行嵌套闭包）
//! - `fetch_dispatch()` 抓取分发循环（transport 级重试 fallback）
//! - `auto_upgrade_check()` Auto 模式升级检查
//!
//! Task 3 重构：EngineContext 多 Spider 共享队列 + callback 路由，process_request
//! 调 `spider.handle()` 而非 `spider.parse()`，items 收集到 `ctx.items`。
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
use tokio::sync::Mutex;
use tracing::Instrument;

use super::stats::SpiderStats;
use super::{
    CrawlEvent, CrawlState, CrawlStats, CrawlStream, Request, Response, Spider, auto, middleware,
    scheduler,
};
use crate::control;
use crate::middleware::{ItemPipeline, Middleware, UaRotationMiddleware};
use crate::observability::events::{EngineEvent, EventBus};
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

#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod tests;

pub use builder::EngineBuilder;
pub(crate) use checkpoint::persist_spider_checkpoint;
pub use config::EngineConfig;
pub(crate) use context::{EngineContext, EngineState, build_crawl_context_for};
pub(crate) use fetch::fetch_dispatch;
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
    /// 自适应并发池（可选）。
    pub autoscale: Option<Arc<AutoscaledPool>>,
    /// 引擎内部事件总线。
    pub event_bus: Arc<EventBus>,
    /// UA 轮换中间件实例（可选）。
    pub ua_middleware: Option<Arc<UaRotationMiddleware>>,
    /// 用户自定义 Engine 级中间件。
    pub custom_middlewares: Vec<Arc<dyn Middleware>>,
    /// 共享 item pipeline。
    pub pipelines: Vec<Arc<dyn ItemPipeline>>,
}

impl Engine {
    /// 运行多个 Spider：共享队列 + callback 路由，每个 Spider 独立 until/stats。
    pub async fn run_many<S: Spider + 'static>(
        &self,
        spiders: Vec<S>,
    ) -> Result<(Vec<CrawlStats>, Vec<serde_json::Value>)> {
        let spiders: Vec<Arc<dyn Spider>> = spiders
            .into_iter()
            .map(|s| Arc::new(s) as Arc<dyn Spider>)
            .collect();
        let items: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let stats = self.run_inner_many(spiders, None, items.clone()).await?;
        let mut guard = items.lock().await;
        let item_list = std::mem::take(&mut *guard);
        Ok((stats, item_list))
    }

    /// 运行单个 Spider。返回 (统计, items)。
    pub async fn run<S: Spider + 'static>(
        &self,
        spider: S,
    ) -> Result<(CrawlStats, Vec<serde_json::Value>)> {
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
        let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
        let engine = self.clone();
        let spiders: Vec<Arc<dyn Spider>> = spiders
            .into_iter()
            .map(|s| Arc::new(s) as Arc<dyn Spider>)
            .collect();
        let driver = Box::pin(run_stream_driver(engine, spiders, tx.clone()));
        let rx = tokio_stream::wrappers::ReceiverStream::new(rx);
        let s = stream::unfold(
            (driver, rx, false),
            async |(mut driver, mut rx, driver_done)| {
                if driver_done {
                    return rx.next().await.map(|e| (e, (driver, rx, true)));
                }
                tokio::select! {
                    biased;
                    event = rx.next() => event.map(|e| (e, (driver, rx, false))),
                    _ = &mut driver => {
                        rx.next().await.map(|e| (e, (driver, rx, true)))
                    }
                }
            },
        );
        CrawlStream { inner: Box::pin(s) }
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
