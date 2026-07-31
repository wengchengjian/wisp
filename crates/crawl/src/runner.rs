//! Engine 运行时：Engine 结构体 + EngineBuilder + run_inner 流驱动。

use futures::stream::{self, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::stats::SpiderStats;
use super::*;
use wisp_core::error::Result;
use wisp_fetcher::{FetchClient, FetchClientConfig};

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
    #[allow(dead_code)] // 多 Spider 阶段暂不持久化 checkpoint，后续计划恢复
    pub(crate) checkpoint_store: Option<Arc<dyn wisp_storage::Store>>,
    #[allow(dead_code)]
    pub(crate) checkpoint_interval: usize,
    /// per-Engine 控制状态。
    pub(crate) control: Arc<control::EngineControl>,
    /// 自适应并发池（可选）。启用后 run_inner 动态调整并发数。
    pub(crate) autoscale: Option<Arc<crate::runtime::autoscale::AutoscaledPool>>,
    /// 运行时并发保护：防止同一 Engine 实例并发调用 run/run_stream。
    /// 未来支持并发爬取时，移除此 guard 并将 EngineControl 改为 per-run 即可。
    pub(crate) running: Arc<AtomicBool>,
    // === 引擎配置（ND-031-ARCH：从 Spider trait 迁移） ===
    /// 抓取模式（Http/Dynamic/Stealth/Auto）。
    pub(crate) fetch_mode: wisp_fetcher::FetchMode,
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
    pub(crate) auto_rules: Vec<(String, wisp_fetcher::FetchMode)>,
}

/// Engine 构造器（Builder 模式）。
pub struct EngineBuilder {
    fetch_client_config: FetchClientConfig,
    max_concurrent: usize,
    max_pages: usize,
    max_refetch_rounds: usize,
    cache_store: Option<Arc<dyn wisp_storage::Store>>,
    checkpoint_store: Option<Arc<dyn wisp_storage::Store>>,
    checkpoint_interval: usize,
    autoscale: Option<Arc<crate::runtime::autoscale::AutoscaledPool>>,
    // === 引擎配置（ND-031-ARCH） ===
    fetch_mode: wisp_fetcher::FetchMode,
    obey_robots: bool,
    max_retries: u32,
    download_delay: Duration,
    headers: Vec<(String, String)>,
    ua_middleware: Option<Arc<crate::middleware::UaRotationMiddleware>>,
    cookie_challenge: bool,
    dynamic_upgrade: bool,
    custom_middlewares: Vec<Arc<dyn crate::middleware::Middleware>>,
    pipelines: Vec<Arc<dyn crate::middleware::ItemPipeline>>,
    auto_rules: Vec<(String, wisp_fetcher::FetchMode)>,
}

/// Engine 公开配置快照（ARCH: master 公开 `EngineConfig` 的轻量移植）。
///
/// 只读视图，由 [`Engine::config`] 生成；内部实现仍以 Engine 字段为事实源。
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// 最大并发数。
    pub max_concurrent: usize,
    /// 最大爬取页数（引擎级兜底）。
    pub max_pages: usize,
    /// 抓取模式。
    pub fetch_mode: wisp_fetcher::FetchMode,
    /// 是否遵守 robots.txt。
    pub obey_robots: bool,
    /// 网络错误重试上限。
    pub max_retries: u32,
    /// 响应中间件 Refetch 最大轮数。
    pub max_refetch_rounds: usize,
    /// 下载延迟。
    pub download_delay: Duration,
    /// 固定请求头。
    pub headers: Vec<(String, String)>,
    /// 是否启用 UA 轮换。
    pub ua_rotation: bool,
    /// 是否启用 Cookie Challenge。
    pub cookie_challenge: bool,
    /// 是否启用 DynamicUpgrade。
    pub dynamic_upgrade: bool,
    /// Auto 模式 URL 规则。
    pub auto_rules: Vec<(String, wisp_fetcher::FetchMode)>,
    /// 是否启用响应缓存。
    pub cache_enabled: bool,
}

impl Engine {
    /// 创建 Engine builder（纯基础设施构造器）。
    ///
    /// 替代原 `Engine::new(spider)` / `Engine::spiders(vec)` / `Engine::builder(spider)`。
    /// Engine 不再持有 Spider，长期持有共享底层资源。
    pub fn infra() -> EngineBuilder {
        EngineBuilder {
            fetch_client_config: FetchClientConfig::default(),
            max_concurrent: 8,
            max_pages: 1000,
            max_refetch_rounds: 5,
            // 响应缓存默认关闭：单次爬取不需要回放，避免每页序列化/反序列化开销。
            cache_store: None,
            checkpoint_store: Some(Arc::new(wisp_storage::FileStore::default())),
            checkpoint_interval: 100,
            autoscale: None,
            // 引擎配置默认值（ND-031-ARCH：原 Spider trait 默认值）
            fetch_mode: wisp_fetcher::FetchMode::Auto,
            obey_robots: true,
            max_retries: 3,
            download_delay: Duration::ZERO,
            headers: Vec::new(),
            ua_middleware: None,
            cookie_challenge: false,
            dynamic_upgrade: false,
            custom_middlewares: Vec::new(),
            pipelines: Vec::new(),
            auto_rules: Vec::new(),
        }
    }

    /// 运行多个 Spider：共享队列 + callback 路由，每个 Spider 独立 until/stats。
    pub async fn run_many<S: Spider + 'static>(
        &self,
        spiders: Vec<S>,
    ) -> Result<(Vec<CrawlStats>, Vec<Value>)> {
        let spiders: Vec<Arc<dyn Spider>> =
            spiders.into_iter().map(|s| Arc::new(s) as Arc<dyn Spider>).collect();
        let items: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let stats = self
            .run_inner_many(spiders, None, items.clone())
            .await?;
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
    pub async fn run<S: Spider + 'static>(&self, spider: S) -> Result<(CrawlStats, Vec<Value>)> {
        let (mut stats, items) = self.run_many(vec![spider]).await?;
        Ok((stats.remove(0), items))
    }

    /// 流式运行：边爬边产出事件（仅单 Spider 模式）。
    ///
    /// # 并发约束
    /// **不可与 `run` 或其他 `run_stream` 并发调用**。共享同一个 `EngineControl`，
    /// 并发会导致 control 状态相互覆盖。需要并发时请创建多个 Engine 实例。
    pub fn run_stream<S: Spider + 'static>(&self, spider: S) -> CrawlStream {
        let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
        let engine = self.clone();
        let driver = async move {
            let items = Arc::new(Mutex::new(Vec::new()));
            let spider: Arc<dyn Spider> = Arc::new(spider);
            match engine
                .run_inner_many(vec![spider], Some(tx.clone()), items)
                .await
            {
                Ok(mut stats) => {
                    let _ = tx.send(CrawlEvent::Done(stats.remove(0))).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(CrawlEvent::Error {
                            url: "*".into(),
                            error: e.to_string(),
                        })
                        .await;
                    let _ = tx.send(CrawlEvent::Done(CrawlStats::default())).await;
                }
            }
        };
        let driver = Box::pin(driver);
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
        }
    }

    /// 关闭 Engine（停止所有运行中的爬取）。
    pub fn shutdown(&self) {
        self.control.shutdown();
    }

    /// 内部运行逻辑：共享队列驱动多个 Spider。
    async fn run_inner_many(
        &self,
        spiders: Vec<Arc<dyn Spider>>,
        tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
        items: Arc<Mutex<Vec<Value>>>,
    ) -> Result<Vec<CrawlStats>> {
        if spiders.is_empty() {
            return Ok(Vec::new());
        }
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(wisp_core::error::WispError::Engine(
                "Engine is already running. Concurrent run/run_stream on the same Engine is not supported. \
                 Create separate Engine instances for concurrent spiders.".into(),
            ));
        }
        struct RunGuard(Arc<AtomicBool>);
        impl Drop for RunGuard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _guard = RunGuard(self.running.clone());

        self.control.reset().await;

        let all_stats: Vec<Arc<SpiderStats>> =
            spiders.iter().map(|_| Arc::new(SpiderStats::new())).collect();
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in &self.auto_rules {
            rule_engine.add_user_rule(pattern, *mode)?;
        }
        let rule_engine = Arc::new(Mutex::new(rule_engine));
        let fetch_mode = self.fetch_mode;
        let max_concurrent = self.max_concurrent;
        let obey_robots = self.obey_robots;
        let fetch_client = self.fetch_client.clone();

        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(robots::RobotsCache::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();

        // 种子请求显式绑定所属 Spider；follow 请求不绑定，由 callback 路由。
        for (i, spider) in spiders.iter().enumerate() {
            for url in spider.start_urls() {
                sched.push(Request::get(&url).with_spider(i)).await;
            }
        }
        for spider in &spiders {
            spider.on_start().await;
        }

        let mw_http_client = fetch_client.http_arc();
        let mw_robots_cache = robots_cache.clone();

        let ctx = Arc::new(engine::EngineContext {
            config: engine::EngineConfig {
                client: fetch_client,
                fetch_mode,
                max_concurrent,
                obey_robots,
                engine_max_pages: self.max_pages,
                max_refetch_rounds: self.max_refetch_rounds,
                max_retries: self.max_retries,
            },
            shared: engine::EngineShared {
                sched: sched.clone(),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                control: self.control.clone(),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: {
                    let defaults = middleware::builtin::default_middlewares(
                        middleware::builtin::DefaultMiddlewareConfig {
                            fetch_mode,
                            delay: self.download_delay,
                            headers: self.headers.clone(),
                            ua_middleware: self.ua_middleware.clone(),
                            cookie_challenge: self.cookie_challenge,
                            dynamic_upgrade: self.dynamic_upgrade,
                            obey_robots,
                            allowed_domains: Default::default(),
                            max_depth: u32::MAX,
                            cache_store: self.cache_store.clone(),
                            http_client: mw_http_client,
                            robots_cache: mw_robots_cache,
                            rule_engine: rule_engine.clone(),
                        },
                    );
                    let mut chain = middleware::MiddlewareChain::new();
                    chain.middlewares = self.custom_middlewares.clone();
                    chain.middlewares.extend(defaults);
                    chain.pipelines = self.pipelines.clone();
                    chain.sort();
                    Arc::new(chain)
                },
                rule_engine,
                cf_domain_locks: Arc::new(dashmap::DashMap::new()),
            },
            state: engine::EngineState {
                spiders,
                all_stats: all_stats.clone(),
                items,
                abort_flag: Arc::new(AtomicBool::new(false)),
                start: std::time::Instant::now(),
                tx,
                global_in_flight: Arc::new(AtomicUsize::new(0)),
            },
        });

        if !ctx.shared.middleware_chain.is_empty() {
            let crawl_ctx = engine::build_crawl_context(&ctx);
            ctx.shared.middleware_chain.run_init(&crawl_ctx).await;
            ctx.shared
                .middleware_chain
                .run_pipelines_open(&crawl_ctx)
                .await;
        }

        let autoscaler_handle = if let Some(ref pool) = self.autoscale {
            pool.set_work_notify(Arc::clone(&ctx.shared.work_notify));
            let pool = Arc::clone(pool);
            let stats = all_stats
                .first()
                .cloned()
                .unwrap_or_else(|| Arc::new(SpiderStats::new()));
            Some(tokio::spawn(async move {
                pool.run_autoscaler(stats).await;
            }))
        } else {
            None
        };

        let stream = {
            let ctx = ctx.clone();
            let autoscale = self.autoscale.clone();
            let buffer_ceiling = if let Some(ref pool) = autoscale {
                pool.max_concurrency()
            } else {
                ctx.config.max_concurrent
            };
            stream::unfold((), move |_| {
                let ctx = ctx.clone();
                let autoscale = autoscale.clone();
                async move {
                    loop {
                        if ctx.shared.control.is_shutdown()
                            || ctx.state.abort_flag.load(Ordering::SeqCst)
                        {
                            return None;
                        }

                        let mut rx_guard = ctx.shared.follow_rx.lock().await;
                        while let Ok(req) = rx_guard.try_recv() {
                            ctx.shared.sched.push(req).await;
                        }
                        drop(rx_guard);

                        let total_pages: usize = ctx
                            .state
                            .all_stats
                            .iter()
                            .map(|s| s.pages.load(Ordering::SeqCst))
                            .sum();
                        if total_pages + ctx.state.global_in_flight.load(Ordering::SeqCst)
                            >= ctx.config.engine_max_pages
                        {
                            if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                return None;
                            }
                            tokio::task::yield_now().await;
                            continue;
                        }

                        let queue_size = ctx.shared.sched.len().await;
                        let limit = if let Some(ref pool) = autoscale {
                            pool.current_concurrency()
                        } else {
                            ctx.config.max_concurrent
                        };
                        if ctx.state.global_in_flight.load(Ordering::SeqCst) >= limit {
                            ctx.shared.work_notify.notified().await;
                            continue;
                        }

                        let mut req = match ctx.shared.sched.pop().await {
                            Some(req) => req,
                            None => {
                                if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                    return None;
                                }
                                ctx.shared.work_notify.notified().await;
                                continue;
                            }
                        };

                        let Some(idx) = ctx.state.spider_index_for(&req) else {
                            tracing::warn!(
                                "丢弃无 Spider 接收的请求: url={}",
                                wisp_core::utils::sanitize_url(&req.url)
                            );
                            continue;
                        };
                        let spider = Arc::clone(&ctx.state.spiders[idx]);
                        let stats = Arc::clone(&ctx.state.all_stats[idx]);
                        let until = spider.until();
                        let stop_ctx = stop::StopContext {
                            pages: stats.pages.load(Ordering::SeqCst),
                            items: stats.items.load(Ordering::SeqCst),
                            errors: stats.errors.load(Ordering::SeqCst),
                            in_flight: stats.in_flight.load(Ordering::SeqCst),
                            elapsed: stats.start.elapsed(),
                            queue_size,
                            callback_pages: if until.uses_callback_pages() {
                                stats.callback_pages_snapshot()
                            } else {
                                HashMap::new()
                            },
                        };
                        if until.should_stop(&stop_ctx) {
                            tracing::info!(
                                "Spider '{}' until() 触发，丢弃后续请求: pages={}, items={}",
                                spider.name(),
                                stop_ctx.pages,
                                stop_ctx.items
                            );
                            continue;
                        }

                        req.spider = Some(idx);
                        ctx.state.global_in_flight.fetch_add(1, Ordering::SeqCst);
                        stats.in_flight.fetch_add(1, Ordering::SeqCst);
                        let ctx_c = ctx.clone();
                        let fut = async move {
                            let _g1 = engine::InFlightGuard {
                                counter: ctx_c.state.global_in_flight.clone(),
                                work_notify: Some(ctx_c.shared.work_notify.clone()),
                            };
                            let _g2 = engine::InFlightGuard {
                                counter: stats.in_flight.clone(),
                                work_notify: None,
                            };
                            if let Some(resp) = engine::process_request(&ctx_c, req).await {
                                engine::process_response(&ctx_c, resp).await;
                            }
                        };
                        return Some((fut, ()));
                    }
                }
            })
            .buffer_unordered(buffer_ceiling)
        };

        tokio::pin!(stream);
        while stream.next().await.is_some() {}

        if let Some(handle) = autoscaler_handle {
            handle.abort();
        }

        if !ctx.shared.middleware_chain.is_empty() {
            let crawl_ctx = engine::build_crawl_context(&ctx);
            ctx.shared
                .middleware_chain
                .run_pipelines_close(&crawl_ctx)
                .await;
        }

        for spider in &ctx.state.spiders {
            spider.on_close().await;
        }

        Ok(ctx
            .state
            .all_stats
            .iter()
            .map(|stats| {
                let status_codes = stats.status_codes_snapshot();
                engine::snapshot_stats_for(stats, status_codes, ctx.state.start)
            })
            .collect())
    }
}

impl EngineBuilder {
    /// 设置最大并发数。
    pub fn max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = n;
        self
    }
    /// 设置最大爬取页数。
    pub fn max_pages(mut self, n: usize) -> Self {
        self.max_pages = n;
        self
    }
    /// 设置 FetchClient 配置（HTTP 连接池/超时/浏览器等基础设施配置，跨 Spider 共享）。
    pub fn fetch_client_config(mut self, config: FetchClientConfig) -> Self {
        self.fetch_client_config = config;
        self
    }
    /// 设置代理（作用于共享 FetchClient 的所有 HTTP 请求）。
    pub fn proxy(mut self, proxy: &str) -> Self {
        self.fetch_client_config.proxy = Some(proxy.to_string());
        self
    }
    /// 设置中间件 Refetch 最大轮数（默认 5）。
    pub fn max_refetch_rounds(mut self, n: usize) -> Self {
        self.max_refetch_rounds = n;
        self
    }
    /// 启用响应缓存（注入 CacheMiddleware，默认 TTL 5 分钟）。
    ///
    /// 默认关闭；需要重复爬取、断点续爬或开发期回放响应时再传入
    /// `MemoryStore` / `FileStore` 等存储后端。
    pub fn cache_store(mut self, store: Arc<dyn wisp_storage::Store>) -> Self {
        self.cache_store = Some(store);
        self
    }
    /// 设置检查点存储（定期保存爬取进度）。
    pub fn checkpoint(mut self, s: Arc<dyn wisp_storage::Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(s);
        self.checkpoint_interval = interval;
        self
    }

    /// 启用自适应并发池。min 为初始/下限，max 为上限。
    /// 启用后 run_inner 会启动后台 autoscaler，根据饱和度动态调整并发数。
    pub fn autoscale(mut self, min: usize, max: usize) -> Self {
        self.autoscale = Some(crate::runtime::autoscale::AutoscaledPool::new(
            min,
            max,
            crate::runtime::autoscale::AutoscaleConfig::default(),
        ));
        self
    }

    /// 同 autoscale(min, max) 但可自定义配置。
    pub fn autoscale_with_config(
        mut self,
        min: usize,
        max: usize,
        config: crate::runtime::autoscale::AutoscaleConfig,
    ) -> Self {
        self.autoscale = Some(crate::runtime::autoscale::AutoscaledPool::new(
            min, max, config,
        ));
        self
    }

    // === 引擎配置方法（ND-031-ARCH：从 Spider trait 迁移） ===

    /// 设置抓取模式（Http/Dynamic/Stealth/Auto，默认 Auto）。
    ///
    /// 这是引擎行为配置，决定如何抓取页面，与 Spider 的解析逻辑无关。
    pub fn fetch_mode(mut self, mode: wisp_fetcher::FetchMode) -> Self {
        self.fetch_mode = mode;
        self
    }

    /// 是否遵守 robots.txt（默认 true）。
    pub fn obey_robots(mut self, obey: bool) -> Self {
        self.obey_robots = obey;
        self
    }

    /// 设置网络错误重试上限（默认 3）。
    ///
    /// fetch_page 失败后，engine 在 fetch_dispatch 内同步重试，计数 `req.retry_count`。
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// 设置下载延迟（默认 0，即无延迟）。
    pub fn download_delay(mut self, d: Duration) -> Self {
        self.download_delay = d;
        self
    }

    /// 设置下载延迟（毫秒）。
    pub fn download_delay_ms(mut self, ms: u64) -> Self {
        self.download_delay = Duration::from_millis(ms);
        self
    }

    /// 设置固定请求头（Engine 级传输能力）。
    pub fn headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    /// 设置 UA 轮换策略；不调用则请求不带 User-Agent。
    pub fn ua_rotation(mut self, ua: crate::middleware::UaRotationMiddleware) -> Self {
        self.ua_middleware = Some(Arc::new(ua));
        self
    }

    /// 是否启用 Cookie Challenge 自动处理。
    pub fn cookie_challenge(mut self, enabled: bool) -> Self {
        self.cookie_challenge = enabled;
        self
    }

    /// Auto 模式是否启用 SPA/DOM 动态升级扫描。
    ///
    /// 默认关闭：该扫描会对每个 200 响应做多模式全量匹配，静态站点会白白浪费性能。
    /// 明确知道目标站点需要 JS 渲染时再开启。
    pub fn dynamic_upgrade(mut self, enabled: bool) -> Self {
        self.dynamic_upgrade = enabled;
        self
    }

    /// 添加自定义 Engine 级中间件。
    pub fn middleware(mut self, mw: Arc<dyn crate::middleware::Middleware>) -> Self {
        self.custom_middlewares.push(mw);
        self
    }

    /// 添加共享 item pipeline（所有 Spider 的 item 汇入同一链）。
    pub fn pipeline(mut self, p: Arc<dyn crate::middleware::ItemPipeline>) -> Self {
        self.pipelines.push(p);
        self
    }

    /// Auto 模式：添加 URL 正则规则（优先级最高，跳过嗅探）。
    ///
    /// 匹配该规则的 URL 直接使用指定模式，不经过 Auto 嗅探。
    pub fn auto_rule(mut self, pattern: &str, mode: wisp_fetcher::FetchMode) -> Self {
        self.auto_rules.push((pattern.to_string(), mode));
        self
    }

    /// 构建引擎实例。
    pub fn build(self) -> Result<Engine> {
        let fetch_client = Arc::new(FetchClient::new(self.fetch_client_config)?);
        Ok(Engine {
            fetch_client,
            cache_store: self.cache_store,
            max_concurrent: self.max_concurrent,
            max_pages: self.max_pages,
            max_refetch_rounds: self.max_refetch_rounds,
            checkpoint_store: self.checkpoint_store,
            checkpoint_interval: self.checkpoint_interval,
            control: Arc::new(control::EngineControl::new()),
            autoscale: self.autoscale,
            running: Arc::new(AtomicBool::new(false)),
            // 引擎配置（ND-031-ARCH）
            fetch_mode: self.fetch_mode,
            obey_robots: self.obey_robots,
            max_retries: self.max_retries,
            download_delay: self.download_delay,
            headers: self.headers,
            ua_middleware: self.ua_middleware,
            cookie_challenge: self.cookie_challenge,
            dynamic_upgrade: self.dynamic_upgrade,
            custom_middlewares: self.custom_middlewares,
            pipelines: self.pipelines,
            auto_rules: self.auto_rules,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_builder_transport_config() {
        let engine = Engine::infra()
            .headers(vec![("Accept".into(), "text/html".into())])
            .ua_rotation(crate::middleware::UaRotationMiddleware::desktop())
            .cookie_challenge(true)
            .build()
            .unwrap();
        assert_eq!(engine.headers.len(), 1);
        assert!(engine.ua_middleware.is_some());
        assert!(engine.cookie_challenge);
        assert!(!engine.dynamic_upgrade, "默认不开启 DynamicUpgrade 扫描");
        assert!(Engine::infra()
            .dynamic_upgrade(true)
            .build()
            .unwrap()
            .dynamic_upgrade);
    }
}
