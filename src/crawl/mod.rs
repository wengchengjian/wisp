//! Spider-based crawling engine.

pub mod scheduler;
pub mod robots;
pub mod cache;
pub mod templates;
pub mod state;
pub mod stats;
pub mod stop;
pub mod items;
pub mod builder;
pub mod session;
pub mod auto;
pub mod engine;
pub mod request_cache;
pub mod control;
pub mod output;
pub mod cron;
pub use state::CrawlState;
pub use items::{Items, JsonlWriter};
pub use builder::{SpiderBuilder, ClosureSpider};
pub use session::{SessionManager, FetcherType};
pub use auto::{SelectorTracker, ModeRuleEngine};
pub use request_cache::RequestCache;
pub use stop::{StopCondition, StopContext, MaxPages, MaxItems, MaxErrors, Timeout, NeverStop, FnStopCondition};

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;

use crate::error::{WispError, Result};
use crate::http::{self, Client};
use crate::parser::{Node, NodeList};
use crate::fetcher::FetchMode;
use self::stats::SpiderStats;

/// HTTP method for spider requests.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Method { Get, Post, Put, Delete }

/// 请求钩子的决策结果。
#[derive(Debug, Clone, PartialEq)]
pub enum RequestAction {
    /// 正常执行
    Proceed,
    /// 跳过此请求
    Skip,
    /// 延迟指定时间后再执行
    Delay(Duration),
    /// 终止整个爬取
    Abort,
}

/// A request to be processed by the spider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiderRequest {
    pub url: String,
    pub method: Method,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    #[serde(skip)]
    pub meta: Value,
    pub callback: Option<String>,
    pub priority: i32,
    /// 深度：起始 URL 为 0，每 follow 一次 +1。
    #[serde(default)]
    pub depth: u32,
}

impl SpiderRequest {
    pub fn get(url: &str) -> Self {
        Self { url: url.to_string(), method: Method::Get, headers: HashMap::new(), body: None, meta: Value::Null, callback: None, priority: 0, depth: 0 }
    }
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self { url: url.to_string(), method: Method::Post, headers: HashMap::new(), body, meta: Value::Null, callback: None, priority: 0, depth: 0 }
    }
    pub fn with_meta(mut self, meta: Value) -> Self { self.meta = meta; self }
    pub fn with_priority(mut self, p: i32) -> Self { self.priority = p; self }
    pub fn with_callback(mut self, cb: &str) -> Self { self.callback = Some(cb.to_string()); self }
    pub fn with_depth(mut self, d: u32) -> Self { self.depth = d; self }
}

/// Response received by the spider.
#[derive(Debug, Clone)]
pub struct SpiderResponse {
    pub url: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub request: SpiderRequest,
    /// Auto 模式选择器追踪器
    #[doc(hidden)]
    pub tracker: Option<Arc<std::sync::Mutex<auto::SelectorTracker>>>,
    /// 是否来自缓存（缓存命中不算 pages_crawled）。
    #[doc(hidden)]
    pub from_cache: bool,
}

impl SpiderResponse {
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone())
            .map_err(|e| WispError::CdpError(format!("utf8 decode: {e}")))
    }
    pub fn parse(&self) -> Result<Node> {
        let text = self.text()?;
        Ok(Node::from_html(&text))
    }
    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.body)
            .map_err(|e| WispError::CdpError(format!("json: {e}")))
    }

    /// 从当前响应 URL 解析相对链接，创建 GET 请求（depth 自动 +1）。
    pub fn follow(&self, href: &str) -> Option<SpiderRequest> {
        let absolute = resolve_href(&self.url, href)?;
        Some(SpiderRequest::get(&absolute).with_depth(self.request.depth + 1))
    }
    pub fn follow_with(&self, href: &str, callback: &str) -> Option<SpiderRequest> {
        let absolute = resolve_href(&self.url, href)?;
        Some(SpiderRequest::get(&absolute).with_callback(callback).with_depth(self.request.depth + 1))
    }
    pub fn follow_meta(&self, href: &str, meta: Value) -> Option<SpiderRequest> {
        let absolute = resolve_href(&self.url, href)?;
        Some(SpiderRequest::get(&absolute).with_meta(meta).with_depth(self.request.depth + 1))
    }

    /// CSS 查询（Auto 模式自动追踪选择器匹配数）。
    pub fn css(&self, sel: &str) -> NodeList {
        let result = self.parse().map(|doc| doc.select(sel)).unwrap_or_else(|_| NodeList::new(vec![]));
        if let Some(ref t) = self.tracker {
            t.lock().unwrap().record(sel, result.len());
        }
        result
    }

    /// XPath 查询（Auto 模式自动追踪）。
    pub fn xpath_auto(&self, expr: &str) -> NodeList {
        let result = self.parse().map(|doc| doc.xpath(expr)).unwrap_or_else(|_| NodeList::new(vec![]));
        if let Some(ref t) = self.tracker {
            t.lock().unwrap().record(expr, result.len());
        }
        result
    }
}

fn resolve_href(base: &str, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let base_url = url::Url::parse(base).ok()?;
    base_url.join(href).ok().map(|u| u.to_string())
}

/// The core Spider trait users implement to define a crawler.
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    // Required
    fn name(&self) -> &str;
    fn start_urls(&self) -> Vec<String>;
    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>);

    /// 请求分发入口。Engine 调用此方法（不直接调 parse）。
    ///
    /// 默认实现：直接调 `parse()`，保持向后兼容。
    /// 用户可重写此方法实现 callback 路由（参考 ClosureSpider）。
    ///
    /// # 路由约定
    /// - `resp.request.callback` 为 `None` 或 `"default"`：入口请求
    /// - 其他字符串：用户自定义 label（通过 `resp.follow_with(url, "detail")` 指定）
    async fn handle(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        self.parse(resp).await
    }

    // Optional with defaults
    fn allowed_domains(&self) -> HashSet<String> { HashSet::new() }
    fn concurrent_requests(&self) -> u32 { 8 }
    fn download_delay(&self) -> Duration { Duration::from_millis(0) }
    fn obey_robots(&self) -> bool { true }
    fn max_retries(&self) -> u32 { 3 }
    fn fetcher_config(&self) -> http::Config { http::Config::default() }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
    async fn on_error(&self, _req: &SpiderRequest, _err: &str) {}
    async fn on_item(&self, item: Value) -> Option<Value> { Some(item) }
    fn is_blocked(&self, resp: &SpiderResponse) -> bool {
        BLOCKED_STATUS_CODES.contains(&resp.status)
    }
    fn configure_sessions(&self, _mgr: &mut session::SessionManager) {}
    fn session_for(&self, _req: &SpiderRequest) -> &str { "default" }
    fn fetch_mode(&self) -> FetchMode { FetchMode::Http }
    fn auto_rules(&self) -> Vec<(String, FetchMode)> { Vec::new() }
    fn auto_exclude(&self) -> HashSet<String> { HashSet::new() }
    /// 最大爬取深度。默认无限制。
    fn max_depth(&self) -> u32 { u32::MAX }
    /// 每次请求随机轮换 User-Agent。
    fn rotate_ua(&self) -> bool { false }
    /// 每个请求执行前的异步钩子。默认返回 Proceed。
    async fn on_before_request(&self, _req: &SpiderRequest) -> RequestAction {
        RequestAction::Proceed
    }

    // === 终止条件（保留） ===

    /// 终止条件。默认永不停止（由引擎 max_pages 兜底）。
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(NeverStop)
    }
}

/// 默认阻塞状态码：401/403/407/429/444/500/502/503/504
pub const BLOCKED_STATUS_CODES: &[u16] = &[401, 403, 407, 429, 444, 500, 502, 503, 504];

/// Crawling statistics.
#[derive(Debug, Clone, Default)]
pub struct CrawlStats {
    pub items_scraped: usize,
    pub pages_crawled: usize,
    pub errors: usize,
    pub duration: Duration,
    pub bytes_downloaded: u64,
    pub avg_response_time: Duration,
    pub domain_counts: HashMap<String, usize>,
    pub blocked_requests: usize,
    pub retry_count: usize,
    pub status_code_counts: HashMap<u16, usize>,
    pub offsite_requests_count: usize,
    pub cache_hits: usize,
}

impl CrawlStats {
    pub fn summary(&self) -> String {
        format!(
            "爬取完成: {} 页 / {} items / {} 错误 / 耗时 {:?} / {:.1} KB / 平均响应 {:?}",
            self.pages_crawled, self.items_scraped, self.errors,
            self.duration, self.bytes_downloaded as f64 / 1024.0, self.avg_response_time
        )
    }
}

/// 爬取过程中的事件流
#[derive(Debug, Clone)]
pub enum CrawlEvent {
    Item(Value),
    PageScraped { url: String, stats: CrawlStats },
    Error { url: String, error: String },
    Done(CrawlStats),
}

/// 流式爬取事件流
pub struct CrawlStream {
    inner: std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>>,
}

impl CrawlStream {
    pub fn items(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = Value>>> {
        use futures::StreamExt;
        Box::pin(self.inner.filter_map(|e| async move {
            match e { CrawlEvent::Item(v) => Some(v), _ => None }
        }))
    }
    pub fn events(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>> {
        self.inner
    }
}

/// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
///
/// Task 3 重构：从"Spider 容器"变为"纯基础设施"。
/// - 不持有 Spider（删除 `spiders: Vec<Box<dyn Spider>>`）
/// - 共享：HTTP client / 代理池 / SQLite 缓存 / RequestCache
/// - 独立：每次 run 内部 Scheduler/去重/stats（per-Spider 隔离）
/// - 控制：per-Engine `EngineControl`（替代原全局 static）
#[derive(Clone)]
pub struct Engine {
    client: Arc<Client>,
    proxy_pool: Option<Arc<crate::proxy::ProxyPool>>,
    cache_store: Option<Arc<crate::storage::Store>>,
    request_cache: Option<RequestCache>,
    max_concurrent: usize,
    max_pages: usize,
    max_depth: Option<u32>,
    dev_mode: bool,
    checkpoint_store: Option<Arc<crate::storage::Store>>,
    checkpoint_interval: usize,
    /// per-Engine 控制状态（替代原全局 static，解决 I4）。
    control: Arc<control::EngineControl>,
}

/// Engine 构造器（Builder 模式）。
pub struct EngineBuilder {
    max_concurrent: usize,
    max_pages: usize,
    max_depth: Option<u32>,
    proxy_pool: Option<Arc<crate::proxy::ProxyPool>>,
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
            proxy_pool: None,
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
    ///
    /// 替代原 `Engine::stream(self)`（消费 self）。新版本接收 `&self` + owned spider，
    /// Engine 可长期持有复用。
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
    ///
    /// 单 Spider：所有 URL 直接给 `ctx.spider` 处理，无路由。
    /// process_request 调 `spider.handle(resp)`（callback 路由），而非 `spider.parse(resp)`。
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
            proxy_pool: self.proxy_pool.clone(),
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
            control: self.control.clone(),
            items,
        });

        // 构建并发流：单 Spider，无路由（删除原 for spider in spiders 循环）
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
                                tokio::task::yield_now().await;
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
    pub fn proxy_pool(mut self, p: Arc<crate::proxy::ProxyPool>) -> Self { self.proxy_pool = Some(p); self }
    pub fn cache_store(mut self, s: Arc<crate::storage::Store>) -> Self { self.cache_store = Some(s); self }
    pub fn request_cache(mut self, c: RequestCache) -> Self { self.request_cache = Some(c); self }
    pub fn dev_mode(mut self, s: Arc<crate::storage::Store>) -> Self {
        self.cache_store = Some(s); self.dev_mode = true; self
    }
    pub fn checkpoint(mut self, s: Arc<crate::storage::Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(s); self.checkpoint_interval = interval; self
    }

    pub fn build(self) -> Result<Engine> {
        let client = Arc::new(
            Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?
        );
        Ok(Engine {
            client,
            proxy_pool: self.proxy_pool,
            cache_store: self.cache_store,
            request_cache: self.request_cache,
            max_concurrent: self.max_concurrent,
            max_pages: self.max_pages,
            max_depth: self.max_depth,
            dev_mode: self.dev_mode,
            checkpoint_store: self.checkpoint_store,
            checkpoint_interval: self.checkpoint_interval,
            control: Arc::new(control::EngineControl::new()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_blocked_status_codes_contains_common_codes() {
        assert!(BLOCKED_STATUS_CODES.contains(&401));
        assert!(BLOCKED_STATUS_CODES.contains(&403));
        assert!(BLOCKED_STATUS_CODES.contains(&407));
        assert!(BLOCKED_STATUS_CODES.contains(&429));
        assert!(BLOCKED_STATUS_CODES.contains(&444));
        assert!(BLOCKED_STATUS_CODES.contains(&500));
        assert!(BLOCKED_STATUS_CODES.contains(&502));
        assert!(BLOCKED_STATUS_CODES.contains(&503));
        assert!(BLOCKED_STATUS_CODES.contains(&504));
        assert!(!BLOCKED_STATUS_CODES.contains(&200));
        assert!(!BLOCKED_STATUS_CODES.contains(&301));
        assert!(!BLOCKED_STATUS_CODES.contains(&404));
    }

    #[test]
    fn test_spider_default_is_blocked_detects_status_codes() {
        struct DummySpider;
        #[async_trait]
        impl Spider for DummySpider {
            fn name(&self) -> &str { "dummy" }
            fn start_urls(&self) -> Vec<String> { vec![] }
            async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) { (vec![], vec![]) }
        }
        let spider = DummySpider;
        let blocked_resp = SpiderResponse {
            url: "http://example.com".into(),
            status: 403,
            headers: HashMap::new(),
            body: vec![],
            request: SpiderRequest::get("http://example.com"),
            tracker: None,
            from_cache: false,
        };
        assert!(spider.is_blocked(&blocked_resp));
        let ok_resp = SpiderResponse { status: 200, ..blocked_resp };
        assert!(!spider.is_blocked(&ok_resp));
    }

    async fn spawn_html_server(html: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(), html
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://{}", addr)
    }

    #[test]
    fn test_crawl_stats_summary() {
        let stats = CrawlStats {
            items_scraped: 10, pages_crawled: 5, errors: 1,
            duration: Duration::from_secs(30), bytes_downloaded: 2048,
            avg_response_time: Duration::from_millis(500),
            domain_counts: { let mut m = HashMap::new(); m.insert("example.com".to_string(), 5); m },
            ..Default::default()
        };
        let s = stats.summary();
        assert!(s.contains("5 页"), "summary 应含页数: {}", s);
        assert!(s.contains("10 items"), "summary 应含 items: {}", s);
        assert!(s.contains("1 错误"), "summary 应含错误数: {}", s);
        assert!(s.contains("2.0 KB"), "summary 应含字节数: {}", s);
    }

    #[test]
    fn test_crawl_stats_default() {
        let stats = CrawlStats::default();
        assert_eq!(stats.items_scraped, 0);
        assert_eq!(stats.bytes_downloaded, 0);
        assert!(stats.domain_counts.is_empty());
        assert_eq!(stats.avg_response_time, Duration::ZERO);
    }

    #[test]
    fn test_crawl_stats_has_status_code_counts() {
        let stats = CrawlStats::default();
        assert!(stats.status_code_counts.is_empty());
    }

    #[test]
    fn test_crawl_stats_has_offsite_requests_count() {
        let stats = CrawlStats::default();
        assert_eq!(stats.offsite_requests_count, 0);
    }

    #[test]
    fn test_crawl_stats_status_code_counts_can_hold_entries() {
        let mut stats = CrawlStats::default();
        stats.status_code_counts.insert(200, 5);
        stats.status_code_counts.insert(404, 1);
        assert_eq!(stats.status_code_counts.get(&200), Some(&5));
        assert_eq!(stats.status_code_counts.get(&404), Some(&1));
    }

    #[tokio::test]
    async fn test_stream_emits_item_and_done() {
        let base = spawn_html_server("<p>1</p>").await;
        struct CountSpider { start_url: String }
        #[async_trait]
        impl Spider for CountSpider {
            fn name(&self) -> &str { "count" }
            fn start_urls(&self) -> Vec<String> { vec![self.start_url.clone()] }
            async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
                let node = resp.parse().unwrap();
                let text = node.select("p").text().join("");
                (vec![serde_json::json!({"text": text})], vec![])
            }
            fn obey_robots(&self) -> bool { false }
        }
        // Task 3：迁移到 Engine::infra().build() + run_stream(spider)
        let engine = Engine::infra().max_pages(1).build().unwrap();
        let mut stream = engine.run_stream(CountSpider { start_url: base }).events();
        let mut items = 0;
        let mut done = false;
        while let Some(event) = stream.next().await {
            match event {
                CrawlEvent::Item(_) => items += 1,
                CrawlEvent::Done(stats) => { assert!(stats.pages_crawled >= 1); done = true; break; }
                _ => {}
            }
        }
        assert!(done, "应收到 Done 事件");
        assert!(items >= 1, "应至少收到 1 个 Item 事件, 实际 {}", items);
    }

    #[tokio::test]
    async fn test_stream_items_helper() {
        let base = spawn_html_server("<p>hello</p>").await;
        struct OneSpider { start_url: String }
        #[async_trait]
        impl Spider for OneSpider {
            fn name(&self) -> &str { "one" }
            fn start_urls(&self) -> Vec<String> { vec![self.start_url.clone()] }
            async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
                (vec![serde_json::json!({"v": 1})], vec![])
            }
            fn obey_robots(&self) -> bool { false }
        }
        // Task 3：迁移到 Engine::infra().build() + run_stream(spider)
        let engine = Engine::infra().max_pages(1).build().unwrap();
        let mut items_stream = engine.run_stream(OneSpider { start_url: base }).items();
        let mut count = 0;
        while items_stream.next().await.is_some() { count += 1; }
        assert!(count >= 1, "items() 应产出至少 1 个 item");
    }
}
