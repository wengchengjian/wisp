//! Engine 运行时：Engine 结构体 + EngineBuilder + run_inner 流驱动。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;
use serde_json::Value;

use crate::error::Result;
use crate::http::Client;
use super::*;
use super::stats::SpiderStats;

/// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
///
/// Task 3 重构：从"Spider 容器"变为"纯基础设施"。
/// - 不持有 Spider（删除 `spiders: Vec<Box<dyn Spider>>`）
/// - 共享：HTTP client / SQLite 缓存 / RequestCache
/// - 独立：每次 run 内部 Scheduler/去重/stats（per-Spider 隔离）
/// - 控制：per-Engine `EngineControl`（替代原全局 static）
#[derive(Clone)]
pub struct Engine {
    pub(crate) client: Arc<Client>,
    pub(crate) cache_store: Option<Arc<crate::storage::Store>>,
    pub(crate) request_cache: Option<RequestCache>,
    pub(crate) max_concurrent: usize,
    pub(crate) max_pages: usize,
    pub(crate) max_depth: Option<u32>,
    pub(crate) max_refetch_rounds: usize,
    pub(crate) dev_mode: bool,
    pub(crate) checkpoint_store: Option<Arc<crate::storage::Store>>,
    pub(crate) checkpoint_interval: usize,
    /// per-Engine 控制状态（替代原全局 static，解决 I4）。
    pub(crate) control: Arc<control::EngineControl>,
}

/// Engine 构造器（Builder 模式）。
pub struct EngineBuilder {
    max_concurrent: usize,
    max_pages: usize,
    max_depth: Option<u32>,
    max_refetch_rounds: usize,
    proxy_url: Option<String>,
    cache_store: Option<Arc<crate::storage::Store>>,
    request_cache: Option<RequestCache>,
    dev_mode: bool,
    checkpoint_store: Option<Arc<crate::storage::Store>>,
    checkpoint_interval: usize,
}

impl Engine {
    /// 创建 Engine builder（纯基础设施构造器）。
    ///
    /// 替代原 `Engine::new(spider)` / `Engine::spiders(vec)` / `Engine::builder(spider)`。
    /// Engine 不再持有 Spider，长期持有共享底层资源。
    pub fn infra() -> EngineBuilder {
        EngineBuilder {
            max_concurrent: 8,
            max_pages: 1000,
            max_depth: None,
            max_refetch_rounds: 5,
            proxy_url: None,
            cache_store: None,
            request_cache: None,
            dev_mode: false,
            checkpoint_store: None,
            checkpoint_interval: 100,
        }
    }

    /// 运行单个 Spider。返回 (统计, items)。
    ///
    /// 共享底层资源（HTTP/缓存/代理），Spider 内部独立 Scheduler/去重。
    /// 可多次调用：`engine.run(spider_a).await?; engine.run(spider_b).await?;`
    ///
    /// 每次调用会重置 `EngineControl`，清理上次的 pause/cancel/shutdown 状态。
    pub async fn run<S: Spider + 'static>(&self, spider: S) -> Result<(CrawlStats, Vec<Value>)> {
        let spider: Arc<dyn Spider> = Arc::new(spider);
        let items: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let stats = self.run_inner(spider, None, items.clone()).await?;
        let items = items.lock().await.clone();
        Ok((stats, items))
    }

    /// 流式运行：边爬边产出事件（仅单 Spider 模式）。
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
                    let _ = tx.send(CrawlEvent::Error { url: "*".into(), error: e.to_string() }).await;
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
        // 重置 control（每次 run 清理上次状态）
        self.control.reset().await;

        let stats = Arc::new(SpiderStats::new());
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in spider.auto_rules() {
            let _ = rule_engine.add_user_rule(&pattern, mode);
        }
        let rule_engine = Arc::new(Mutex::new(rule_engine));
        let allowed = Arc::new(spider.allowed_domains());
        let fetcher_config = spider.fetcher_config();
        let fetch_mode = spider.fetch_mode();
        let max_concurrent = self.max_concurrent;
        let max_depth = self.max_depth.unwrap_or_else(|| spider.max_depth());
        let obey_robots = spider.obey_robots();
        let auto_excludes = spider.auto_exclude();

        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(Mutex::new(robots::RobotsCache::new()));
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();

        // checkpoint 恢复（单 Spider）
        let spider_name = spider.name().to_string();
        let mut restored_pending = false;
        if let Some(ref store) = self.checkpoint_store {
            if let Some(blob) = store.load_checkpoint(&spider_name)? {
                match bincode::deserialize::<CrawlState>(&blob) {
                    Ok(state) => {
                        if !state.pending_urls.is_empty() {
                            let n = state.pending_urls.len();
                            for req in state.pending_urls {
                                sched.push(req).await;
                            }
                            tracing::info!(
                                "Spider '{}' 从 checkpoint 恢复 {} 个 pending URLs",
                                spider_name, n
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
                sched.push(SpiderRequest::get(&url)).await;
            }
        }

        spider.on_start().await;

        let ctx = Arc::new(engine::EngineContext {
            client: self.client.clone(),
            sched: sched.clone(),
            robots_cache,
            follow_tx,
            follow_rx: Arc::new(Mutex::new(follow_rx)),
            domain_sems: Arc::new(Mutex::new(HashMap::new())),
            proxy_clients: Arc::new(Mutex::new(HashMap::new())),
            cache_store: self.cache_store.clone(),
            request_cache: self.request_cache.clone(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            start: std::time::Instant::now(),
            tx,
            dev_mode: self.dev_mode,
            spider: spider.clone(),
            stats: stats.clone(),
            rule_engine,
            auto_excludes,
            allowed,
            fetcher_config,
            fetch_mode,
            max_concurrent,
            max_depth,
            obey_robots,
            global_in_flight: Arc::new(AtomicUsize::new(0)),
            engine_max_pages: self.max_pages,
            max_refetch_rounds: self.max_refetch_rounds,
            control: self.control.clone(),
            items,
            work_notify: Arc::new(tokio::sync::Notify::new()),
            middleware_chain: {
                let mut chain = middleware::MiddlewareChain::new();
                chain.middlewares = spider.middlewares();
                chain.pipelines = spider.pipelines();
                chain.sort(); // 按 priority 排序
                Arc::new(chain)
            },
        });

        // 中间件初始化：在爬取开始前调用所有中间件的 init
        if !ctx.middleware_chain.is_empty() {
            let crawl_ctx = engine::build_crawl_context(&ctx);
            ctx.middleware_chain.run_init(&crawl_ctx).await;
        }

        // 构建并发流：单 Spider，无路由
        let stream = {
            let ctx = ctx.clone();
            let max_concurrent = ctx.max_concurrent;
            stream::unfold((), move |_| {
                let ctx = ctx.clone();
                async move {
                    loop {
                        if ctx.control.is_shutdown() || ctx.abort_flag.load(Ordering::SeqCst) {
                            return None;
                        }

                        // drain follow channel
                        let mut rx_guard = ctx.follow_rx.lock().await;
                        while let Ok(req) = rx_guard.try_recv() {
                            ctx.sched.push(req).await;
                        }
                        drop(rx_guard);

                        // 引擎级 max_pages 兜底
                        let pages = ctx.stats.pages.load(Ordering::SeqCst);
                        if pages + ctx.global_in_flight.load(Ordering::SeqCst) >= ctx.engine_max_pages {
                            if ctx.global_in_flight.load(Ordering::SeqCst) == 0 { return None; }
                            tokio::task::yield_now().await;
                            continue;
                        }

                        // Spider until 终止条件检查
                        let queue_size = ctx.sched.len().await;
                        let stop_ctx = stop::StopContext {
                            pages: ctx.stats.pages.load(Ordering::SeqCst),
                            items: ctx.stats.items.load(Ordering::SeqCst),
                            errors: ctx.stats.errors.load(Ordering::SeqCst),
                            in_flight: ctx.stats.in_flight.load(Ordering::SeqCst),
                            elapsed: ctx.stats.start.elapsed(),
                            queue_size,
                        };
                        if ctx.spider.until().should_stop(&stop_ctx) {
                            if ctx.global_in_flight.load(Ordering::SeqCst) == 0 { return None; }
                            tokio::task::yield_now().await;
                            continue;
                        }

                        let req = match ctx.sched.pop().await {
                            Some(req) => req,
                            None => {
                                if ctx.global_in_flight.load(Ordering::SeqCst) == 0 { return None; }
                                tokio::time::timeout(Duration::from_millis(100), ctx.work_notify.notified()).await.ok();
                                continue;
                            }
                        };

                        // 单 Spider：直接派发，无路由
                        ctx.global_in_flight.fetch_add(1, Ordering::SeqCst);
                        ctx.stats.in_flight.fetch_add(1, Ordering::SeqCst);
                        let ctx_c = ctx.clone();
                        let fut = async move {
                            let _g1 = engine::InFlightGuard { counter: ctx_c.global_in_flight.clone() };
                            let _g2 = engine::InFlightGuard { counter: ctx_c.stats.in_flight.clone() };
                            engine::process_request(&ctx_c, req).await;
                        };
                        return Some((fut, ()));
                    }
                }
            })
            .buffer_unordered(max_concurrent)
        };

        // 驱动流 + 定期 checkpoint
        tokio::pin!(stream);
        let mut pages_since_checkpoint = 0usize;
        while stream.next().await.is_some() {
            pages_since_checkpoint += 1;
            if pages_since_checkpoint >= self.checkpoint_interval {
                if let Some(ref store) = self.checkpoint_store {
                    engine::save_checkpoint(store, &spider_name, &sched, &ctx.stats).await;
                }
                pages_since_checkpoint = 0;
            }
        }

        spider.on_close().await;

        if let Some(ref store) = self.checkpoint_store {
            if let Err(e) = store.delete_checkpoint(&spider_name) {
                tracing::warn!("删除 checkpoint 失败: {}", e);
            }
        }

        let status_codes = ctx.stats.status_codes.lock().await.clone();
        Ok(engine::snapshot_stats_for(&ctx.stats, status_codes, ctx.start))
    }
}

impl EngineBuilder {
    pub fn max_concurrent(mut self, n: usize) -> Self { self.max_concurrent = n; self }
    pub fn max_pages(mut self, n: usize) -> Self { self.max_pages = n; self }
    pub fn max_depth(mut self, n: u32) -> Self { self.max_depth = Some(n); self }
    /// 设置中间件 Refetch 最大轮数（默认 5）。
    pub fn max_refetch_rounds(mut self, n: usize) -> Self { self.max_refetch_rounds = n; self }
    /// 设置固定 HTTP 代理（如 "http://127.0.0.1:7897"）。
    pub fn proxy(mut self, url: &str) -> Self { self.proxy_url = Some(url.to_string()); self }
    pub fn cache_store(mut self, s: Arc<crate::storage::Store>) -> Self { self.cache_store = Some(s); self }
    pub fn request_cache(mut self, c: RequestCache) -> Self { self.request_cache = Some(c); self }
    pub fn dev_mode(mut self, s: Arc<crate::storage::Store>) -> Self {
        self.cache_store = Some(s); self.dev_mode = true; self
    }
    pub fn checkpoint(mut self, s: Arc<crate::storage::Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(s); self.checkpoint_interval = interval; self
    }

    pub fn build(self) -> Result<Engine> {
        let mut builder = Client::builder()
            .timeout(std::time::Duration::from_secs(30));
        if let Some(ref proxy) = self.proxy_url {
            builder = builder.proxy(proxy);
        }
        let client = Arc::new(builder.build()?);
        Ok(Engine {
            client,
            cache_store: self.cache_store,
            request_cache: self.request_cache,
            max_concurrent: self.max_concurrent,
            max_pages: self.max_pages,
            max_depth: self.max_depth,
            max_refetch_rounds: self.max_refetch_rounds,
            dev_mode: self.dev_mode,
            checkpoint_store: self.checkpoint_store,
            checkpoint_interval: self.checkpoint_interval,
            control: Arc::new(control::EngineControl::new()),
        })
    }
}
