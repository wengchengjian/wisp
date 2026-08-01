//! Engine 基础设施主体。

use futures::stream::{self, StreamExt};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::config::EngineConfig;
use super::run_stream_driver;
use crate::control;
use crate::observability::events::EventBus;
use crate::runtime::autoscale::AutoscaledPool;
use crate::{CrawlEvent, CrawlStats, CrawlStream, Spider};
use wisp_core::error::Result;
use wisp_fetcher::{FetchClient, FetchMode};

/// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
///
/// Task 3 重构：从"Spider 容器"变为"纯基础设施"。
/// - 不持有 Spider（删除 `spiders: Vec<Box<dyn Spider>>`）
/// - 共享：HTTP client / Store（SQLite 缓存 + checkpoint）
/// - 独立：每次 run 内部 Scheduler/去重/stats（per-Spider 隔离）
/// - 控制：per-Engine `EngineControl`
///
/// ND-031-ARCH 修复：引擎配置（fetch_mode/obey_robots/max_retries/download_delay/auto_rules）
/// 从 Spider trait 迁移到 Engine，职责分离：Spider 只关心解析逻辑，Engine 管理抓取行为。
#[derive(Clone)]
pub struct Engine {
    /// 共享 FetchClient（HTTP 连接池 + BrowserPool，跨 Spider 复用）
    pub(crate) fetch_client: Arc<FetchClient>,
    pub(crate) cache_store: Option<Arc<dyn wisp_storage::Store>>,
    pub(crate) max_concurrent: usize,
    pub(crate) max_pages: usize,
    pub(crate) max_refetch_rounds: usize,
    /// checkpoint 存储（可选，定期保存爬取进度）。
    pub(crate) checkpoint_store: Option<Arc<dyn wisp_storage::Store>>,
    /// checkpoint 保存间隔（页数）。
    pub(crate) checkpoint_interval: usize,
    /// per-Engine 控制状态。
    pub(crate) control: Arc<control::EngineControl>,
    /// 自适应并发池（可选）。启用后 run_inner 动态调整并发数。
    pub(crate) autoscale: Option<Arc<AutoscaledPool>>,
    /// 运行时并发保护：防止同一 Engine 实例并发调用 run/run_stream。
    /// 未来支持并发爬取时，移除此 guard 并将 EngineControl 改为 per-run 即可。
    pub(crate) running: Arc<AtomicBool>,
    /// 引擎内部事件总线（无监听器时零成本 no-op）。
    pub(crate) event_bus: Arc<EventBus>,
    // === 引擎配置（ND-031-ARCH：从 Spider trait 迁移） ===
    /// 抓取模式（Http/Dynamic/Stealth/Auto）。
    pub(crate) fetch_mode: FetchMode,
    /// 是否遵守 robots.txt。
    pub(crate) obey_robots: bool,
    /// 网络错误重试上限（fetch 失败后同步重试）。
    pub(crate) max_retries: u32,
    /// 下载延迟（每次请求前的等待时间）。
    pub(crate) download_delay: Duration,
    /// 固定请求头（Engine 级传输能力）。
    pub(crate) headers: Vec<(String, String)>,
    /// UA 轮换策略（Engine 级传输能力）。
    pub(crate) ua_middleware: Option<Arc<crate::middleware::UaRotationMiddleware>>,
    /// 是否启用 Cookie Challenge 自动处理。
    pub(crate) cookie_challenge: bool,
    /// Auto 模式是否启用 SPA/DOM 动态升级扫描（默认关闭，避免每页全量扫描）。
    pub(crate) dynamic_upgrade: bool,
    /// 用户自定义 Engine 级中间件。
    pub(crate) custom_middlewares: Vec<Arc<dyn crate::middleware::Middleware>>,
    /// 共享 item pipeline（Engine 级，所有 Spider 汇入同一链）。
    pub(crate) pipelines: Vec<Arc<dyn crate::middleware::ItemPipeline>>,
    /// Auto 模式 URL 正则规则（优先级最高，跳过嗅探）。
    pub(crate) auto_rules: Vec<(String, FetchMode)>,
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
    ///
    /// 共享底层资源（HTTP/缓存/代理），Spider 内部独立 Scheduler/去重。
    /// 可多次调用：`engine.run(spider_a).await?; engine.run(spider_b).await?;`
    ///
    /// # 并发约束
    /// **不可并发调用**。`run` / `run_stream` 共享同一个 `EngineControl`，
    /// 并发调用会导致 control 状态（pause/cancel/shutdown）相互覆盖。
    /// 需要并发爬取多个 Spider 时，请为每个 Spider 创建独立的 Engine 实例。
    ///
    /// 每次调用会重置 `EngineControl`，清理上次的 pause/cancel/shutdown 状态。
    ///
    /// # Errors
    ///
    /// - `NetworkError::Http` — 同一 Engine 实例并发调用 run/run_stream。
    /// - 其他错误由 Spider 回调或中间件产生。
    pub async fn run<S: Spider + 'static>(
        &self,
        spider: S,
    ) -> Result<(CrawlStats, Vec<serde_json::Value>)> {
        let (mut stats, items) = self.run_many(vec![spider]).await?;
        Ok((stats.remove(0), items))
    }

    /// 流式运行：边爬边产出事件（仅单 Spider 模式）。
    ///
    /// # 并发约束
    /// **不可与 `run` 或其他 `run_stream` 并发调用**。共享同一个 `EngineControl`，
    /// 并发会导致 control 状态相互覆盖。需要并发时请创建多个 Engine 实例。
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
    ///
    /// # 并发约束
    /// **不可与 `run` 或其他 `run_stream` 并发调用**。共享同一个 `EngineControl`，
    /// 并发会导致 control 状态相互覆盖。需要并发时请创建多个 Engine 实例。
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
            |(mut driver, mut rx, driver_done)| async move {
                if driver_done {
                    return rx.next().await.map(|e| (e, (driver, rx, true)));
                }
                tokio::select! {
                    biased;
                    event = rx.next() => match event {
                        Some(e) => Some((e, (driver, rx, false))),
                        None => None,
                    },
                    _ = &mut driver => {
                        match rx.next().await {
                            Some(e) => Some((e, (driver, rx, true))),
                            None => None,
                        }
                    }
                }
            },
        );
        CrawlStream { inner: Box::pin(s) }
    }

    /// 获取控制句柄（用于外部 pause/resume/cancel/shutdown）。
    pub fn control(&self) -> &Arc<control::EngineControl> {
        &self.control
    }

    /// 获取 Engine 公开配置快照。
    #[must_use]
    pub fn config(&self) -> EngineConfig {
        EngineConfig {
            max_concurrent: self.max_concurrent,
            transport: self.fetch_client.config().clone(),
            max_pages: self.max_pages,
            fetch_mode: self.fetch_mode,
            obey_robots: self.obey_robots,
            max_retries: self.max_retries,
            max_refetch_rounds: self.max_refetch_rounds,
            download_delay: self.download_delay,
            headers: self.headers.clone(),
            ua_rotation: self.ua_middleware.is_some(),
            cookie_challenge: self.cookie_challenge,
            dynamic_upgrade: self.dynamic_upgrade,
            auto_rules: self.auto_rules.clone(),
            cache_enabled: self.cache_store.is_some(),
            checkpoint_enabled: self.checkpoint_store.is_some(),
            checkpoint_interval: self.checkpoint_interval,
        }
    }

    /// 关闭 Engine（停止所有运行中的爬取）。
    pub fn shutdown(&self) {
        self.control.shutdown();
    }
}
