//! Engine 运行时：Engine 结构体 + EngineBuilder + run_inner 流驱动。

use futures::stream::{self, StreamExt};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::stats::SpiderStats;
use super::*;
use crate::error::Result;
use crate::fetcher::{FetchClient, FetchClientConfig};

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
    pub(crate) cache_store: Option<Arc<dyn crate::storage::Store>>,
    pub(crate) max_concurrent: usize,
    pub(crate) max_pages: usize,
    pub(crate) max_refetch_rounds: usize,
    pub(crate) checkpoint_store: Option<Arc<dyn crate::storage::Store>>,
    pub(crate) checkpoint_interval: usize,
    /// per-Engine 控制状态。
    pub(crate) control: Arc<control::EngineControl>,
    /// 自适应并发池（可选）。启用后 run_inner 动态调整并发数。
    pub(crate) autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
    /// 运行时并发保护：防止同一 Engine 实例并发调用 run/run_stream。
    /// 未来支持并发爬取时，移除此 guard 并将 EngineControl 改为 per-run 即可。
    pub(crate) running: Arc<AtomicBool>,
    // === 引擎配置（ND-031-ARCH：从 Spider trait 迁移） ===
    /// 抓取模式（Http/Dynamic/Stealth/Auto）。
    pub(crate) fetch_mode: crate::fetcher::FetchMode,
    /// 是否遵守 robots.txt。
    pub(crate) obey_robots: bool,
    /// 网络错误重试上限（fetch 失败后同步重试）。
    pub(crate) max_retries: u32,
    /// 下载延迟（每次请求前的等待时间）。
    pub(crate) download_delay: Duration,
    /// Auto 模式 URL 正则规则（优先级最高，跳过嗅探）。
    pub(crate) auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
}

/// Engine 构造器（Builder 模式）。
pub struct EngineBuilder {
    fetch_client_config: FetchClientConfig,
    max_concurrent: usize,
    max_pages: usize,
    max_refetch_rounds: usize,
    cache_store: Option<Arc<dyn crate::storage::Store>>,
    checkpoint_store: Option<Arc<dyn crate::storage::Store>>,
    checkpoint_interval: usize,
    autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
    // === 引擎配置（ND-031-ARCH） ===
    fetch_mode: crate::fetcher::FetchMode,
    obey_robots: bool,
    max_retries: u32,
    download_delay: Duration,
    auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
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
            cache_store: Some(Arc::new(crate::storage::MemoryStore::default())),
            checkpoint_store: Some(Arc::new(crate::storage::FileStore::default())),
            checkpoint_interval: 100,
            autoscale: None,
            // 引擎配置默认值（ND-031-ARCH：原 Spider trait 默认值）
            fetch_mode: crate::fetcher::FetchMode::Auto,
            obey_robots: true,
            max_retries: 3,
            download_delay: Duration::ZERO,
            auto_rules: Vec::new(),
        }
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
        let spider: Arc<dyn Spider> = Arc::new(spider);
        let items: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let stats = self.run_inner(spider, None, items.clone()).await?;
        let items = items.lock().await.clone();
        Ok((stats, items))
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
            match engine.run_inner(spider, Some(tx.clone()), items).await {
                Ok(stats) => {
                    let _ = tx.send(CrawlEvent::Done(stats)).await;
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

    /// 关闭 Engine（停止所有运行中的爬取）。
    pub fn shutdown(&self) {
        self.control.shutdown();
    }

    /// 内部运行逻辑：构建 ctx + 驱动流 + 汇总 stats。
    async fn run_inner(
        &self,
        spider: Arc<dyn Spider>,
        tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
        items: Arc<Mutex<Vec<Value>>>,
    ) -> Result<CrawlStats> {
        // 运行时并发保护：同一 Engine 实例不允许并发 run。
        // 未来支持并发时移除此 guard，将 EngineControl 改为 per-run 即可。
        // ND-001-ARCH：使用语义正确的 WispError::Engine 变体，而非 NetworkError::Http。
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(crate::error::WispError::Engine(
                "Engine is already running. Concurrent run/run_stream on the same Engine is not supported. \
                 Create separate Engine instances for concurrent spiders.".into(),
            ));
        }
        // RAII guard：无论正常结束还是 panic，都释放 running 标志
        struct RunGuard(Arc<AtomicBool>);
        impl Drop for RunGuard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _guard = RunGuard(self.running.clone());

        // 重置 control（每次 run 清理上次状态）
        self.control.reset().await;

        let stats = Arc::new(SpiderStats::new());
        // ND-031-ARCH：引擎配置从 Engine 自身读取（而非 Spider trait 方法）
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in &self.auto_rules {
            rule_engine.add_user_rule(pattern, *mode)?;
        }
        let rule_engine = Arc::new(Mutex::new(rule_engine));
        let fetch_mode = self.fetch_mode;
        let max_concurrent = self.max_concurrent;
        let max_depth = spider.max_depth();
        let obey_robots = self.obey_robots;

        // 复用 Engine 持有的共享 FetchClient（HTTP 连接池 + BrowserPool 跨 Spider 复用）
        let fetch_client = self.fetch_client.clone();

        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(robots::RobotsCache::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();

        // checkpoint 恢复（单 Spider）
        let spider_name = spider.name().to_string();
        let mut restored_pending = false;
        if let Some(ref store) = self.checkpoint_store {
            if let Some(blob) = crate::storage::load_checkpoint(&**store, &spider_name)? {
                match bincode::deserialize::<CrawlState>(&blob) {
                    Ok(state) => {
                        if !state.pending_urls.is_empty() {
                            let n = state.pending_urls.len();
                            // 用 restore 一次性恢复 pending + seen 去重集合，
                            // 避免逐个 push 时已爬 URL 因 seen 丢失被重新入队。
                            let seen = state.seen_urls.clone();
                            sched.restore(state.pending_urls, seen).await;
                            tracing::info!(
                                "Spider '{}' 从 checkpoint 恢复 {} 个 pending URLs (含 {} seen)",
                                spider_name,
                                n,
                                sched.seen_urls().await.len()
                            );
                            restored_pending = true;
                        }
                    }
                    Err(e) => tracing::warn!("checkpoint 反序列化失败: {}", e),
                }
            }
        }

        if !restored_pending {
            for url in spider.start_urls() {
                sched.push(Request::get(&url)).await;
            }
        }

        spider.on_start().await;

        // 默认中间件注入所需资源（在 ctx 字面量 move 这些 Arc 前提取）
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
                // ND-009-SEC：moka::Cache 限制 proxy client 缓存最大 1024 条
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                control: self.control.clone(),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: {
                    // 默认中间件链：按 fetch_mode + spider 配置注入（详见 builtin::default_middlewares）
                    let defaults = middleware::builtin::default_middlewares(
                        middleware::builtin::DefaultMiddlewareConfig {
                            fetch_mode,
                            delay: self.download_delay,
                            obey_robots,
                            allowed_domains: spider.allowed_domains(),
                            max_depth,
                            cache_store: self.cache_store.clone(),
                            http_client: mw_http_client,
                            robots_cache: mw_robots_cache,
                            rule_engine: rule_engine.clone(),
                        },
                    );
                    let mut chain = middleware::MiddlewareChain::new();
                    // 用户中间件 + 默认中间件合并，sort 按 priority 统一排序
                    chain.middlewares = spider.middlewares();
                    chain.middlewares.extend(defaults);
                    chain.pipelines = spider.pipelines();
                    chain.sort();
                    Arc::new(chain)
                },
                rule_engine,
                cf_domain_locks: Arc::new(dashmap::DashMap::new()),
            },
            state: engine::EngineState {
                spider: spider.clone(),
                stats: stats.clone(),
                items,
                abort_flag: Arc::new(AtomicBool::new(false)),
                start: std::time::Instant::now(),
                tx,
                global_in_flight: Arc::new(AtomicUsize::new(0)),
            },
        });

        // 中间件初始化：在爬取开始前调用所有中间件的 init + pipeline 的 open
        if !ctx.shared.middleware_chain.is_empty() {
            let crawl_ctx = engine::build_crawl_context(&ctx);
            ctx.shared.middleware_chain.run_init(&crawl_ctx).await;
            ctx.shared
                .middleware_chain
                .run_pipelines_open(&crawl_ctx)
                .await;
        }

        // 启用 autoscale 时，spawn 后台 autoscaler task
        let autoscaler_handle = if let Some(ref pool) = self.autoscale {
            // ND-004-CORR：注入 work_notify，autoscale 扩容时唤醒主循环，
            // 避免主循环 10ms timeout 轮询。
            pool.set_work_notify(Arc::clone(&ctx.shared.work_notify));
            let pool = Arc::clone(pool);
            let stats = Arc::clone(&stats);
            Some(tokio::spawn(async move {
                pool.run_autoscaler(stats).await;
            }))
        } else {
            None
        };

        // 构建并发流：单 Spider，无路由
        let stream = {
            let ctx = ctx.clone();
            let autoscale = self.autoscale.clone();
            // buffer_unordered 的 ceiling：autoscale 启用时用 max_concurrency()，否则用 max_concurrent
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

                        // drain follow channel
                        let mut rx_guard = ctx.shared.follow_rx.lock().await;
                        while let Ok(req) = rx_guard.try_recv() {
                            ctx.shared.sched.push(req).await;
                        }
                        drop(rx_guard);

                        // 引擎级 max_pages 兜底
                        let pages = ctx.state.stats.pages.load(Ordering::SeqCst);
                        if pages + ctx.state.global_in_flight.load(Ordering::SeqCst)
                            >= ctx.config.engine_max_pages
                        {
                            if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                return None;
                            }
                            tokio::task::yield_now().await;
                            continue;
                        }

                        // Spider until 终止条件检查
                        let queue_size = ctx.shared.sched.len().await;
                        let stop_ctx = stop::StopContext {
                            pages: ctx.state.stats.pages.load(Ordering::SeqCst),
                            items: ctx.state.stats.items.load(Ordering::SeqCst),
                            errors: ctx.state.stats.errors.load(Ordering::SeqCst),
                            in_flight: ctx.state.stats.in_flight.load(Ordering::SeqCst),
                            elapsed: ctx.state.stats.start.elapsed(),
                            queue_size,
                        };
                        if ctx.state.spider.until().should_stop(&stop_ctx) {
                            if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                tracing::info!(
                                    "Spider until() 终止条件触发，停止派发: pages={}, items={}, queue={}",
                                    stop_ctx.pages,
                                    stop_ctx.items,
                                    stop_ctx.queue_size
                                );
                                return None;
                            }
                            tokio::task::yield_now().await;
                            continue;
                        }

                        // 动态并发限制：autoscale 启用时检查 current_concurrency
                        let limit = if let Some(ref pool) = autoscale {
                            pool.current_concurrency()
                        } else {
                            ctx.config.max_concurrent
                        };
                        if ctx.state.global_in_flight.load(Ordering::SeqCst) >= limit {
                            // ND-004-CORR/ND-007-PERF：已达并发上限，纯 Notify 驱动等待。
                            // 唤醒来源：process_response 末尾 notify_one（in-flight 下降）、
                            // autoscaler 扩容时 notify_one（limit 上升）。
                            // 不再使用 10ms timeout 轮询，避免 CPU 浪费。
                            ctx.shared.work_notify.notified().await;
                            continue;
                        }

                        let req = match ctx.shared.sched.pop().await {
                            Some(req) => req,
                            None => {
                                if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                    return None;
                                }
                                // ND-004-CORR/ND-007-PERF：scheduler 空但仍有 in-flight，
                                // 纯 Notify 驱动等待新 work（follow 请求通过 process_response notify）。
                                ctx.shared.work_notify.notified().await;
                                continue;
                            }
                        };

                        // 单 Spider：直接派发，无路由
                        ctx.state.global_in_flight.fetch_add(1, Ordering::SeqCst);
                        ctx.state.stats.in_flight.fetch_add(1, Ordering::SeqCst);
                        let ctx_c = ctx.clone();
                        let fut = async move {
                            let _g1 = engine::InFlightGuard {
                                counter: ctx_c.state.global_in_flight.clone(),
                                work_notify: ctx_c.shared.work_notify.clone(),
                            };
                            let _g2 = engine::InFlightGuard {
                                counter: ctx_c.state.stats.in_flight.clone(),
                                work_notify: ctx_c.shared.work_notify.clone(),
                            };
                            // 请求阶段 → 响应阶段（同级编排，process_request 不再内嵌 process_response）
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

        // 驱动流 + 定期 checkpoint
        tokio::pin!(stream);
        let mut pages_since_checkpoint = 0usize;
        while stream.next().await.is_some() {
            pages_since_checkpoint += 1;
            if pages_since_checkpoint >= self.checkpoint_interval {
                if let Some(ref store) = self.checkpoint_store {
                    // ND-003-ERR：save_checkpoint 失败时发送 Error 事件，不静默吞掉
                    if let Err(e) = engine::persist_spider_checkpoint(
                        store.as_ref(),
                        &spider_name,
                        &sched,
                        &ctx.state.stats,
                    )
                    .await
                    {
                        tracing::warn!("checkpoint 失败: {}", e);
                        if let Some(ref tx) = ctx.state.tx {
                            let _ = tx.try_send(CrawlEvent::Error {
                                url: String::new(),
                                error: format!("checkpoint failed: {e}"),
                            });
                        }
                    }
                }
                pages_since_checkpoint = 0;
            }
        }

        // abort autoscaler 后台 task
        if let Some(handle) = autoscaler_handle {
            handle.abort();
        }

        // pipeline 关闭：爬取结束后释放资源
        if !ctx.shared.middleware_chain.is_empty() {
            let crawl_ctx = engine::build_crawl_context(&ctx);
            ctx.shared
                .middleware_chain
                .run_pipelines_close(&crawl_ctx)
                .await;
        }

        spider.on_close().await;

        if let Some(ref store) = self.checkpoint_store {
            if let Err(e) = crate::storage::delete_checkpoint(&**store, &spider_name) {
                tracing::warn!("删除 checkpoint 失败: {}", e);
            }
        }

        let status_codes = ctx.state.stats.status_codes_snapshot();
        Ok(engine::snapshot_stats_for(
            &ctx.state.stats,
            status_codes,
            ctx.state.start,
        ))
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
    /// 设置响应缓存存储（注入 CacheMiddleware，永不过期）。
    /// 想要 TTL 的用户应通过 `Spider::middlewares()` 自定义 `CacheMiddleware`。
    pub fn cache_store(mut self, store: Arc<dyn crate::storage::Store>) -> Self {
        self.cache_store = Some(store);
        self
    }
    /// 设置检查点存储（定期保存爬取进度）。
    pub fn checkpoint(mut self, s: Arc<dyn crate::storage::Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(s);
        self.checkpoint_interval = interval;
        self
    }

    /// 启用自适应并发池。min 为初始/下限，max 为上限。
    /// 启用后 run_inner 会启动后台 autoscaler，根据饱和度动态调整并发数。
    pub fn autoscale(mut self, min: usize, max: usize) -> Self {
        self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(
            min,
            max,
            crate::crawl::runtime::autoscale::AutoscaleConfig::default(),
        ));
        self
    }

    /// 同 autoscale(min, max) 但可自定义配置。
    pub fn autoscale_with_config(
        mut self,
        min: usize,
        max: usize,
        config: crate::crawl::runtime::autoscale::AutoscaleConfig,
    ) -> Self {
        self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(
            min, max, config,
        ));
        self
    }

    // === 引擎配置方法（ND-031-ARCH：从 Spider trait 迁移） ===

    /// 设置抓取模式（Http/Dynamic/Stealth/Auto，默认 Auto）。
    ///
    /// 这是引擎行为配置，决定如何抓取页面，与 Spider 的解析逻辑无关。
    pub fn fetch_mode(mut self, mode: crate::fetcher::FetchMode) -> Self {
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

    /// Auto 模式：添加 URL 正则规则（优先级最高，跳过嗅探）。
    ///
    /// 匹配该规则的 URL 直接使用指定模式，不经过 Auto 嗅探。
    pub fn auto_rule(mut self, pattern: &str, mode: crate::fetcher::FetchMode) -> Self {
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
            auto_rules: self.auto_rules,
        })
    }
}
