//! Spider-based crawling engine.

pub mod scheduler;
pub mod robots;
pub mod cache;
pub mod templates;
pub mod state;
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

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    /// Cron 表达式（标准 5 字段）。返回 None 表示立即执行一次（默认行为）。
    fn schedule(&self) -> Option<&str> { None }
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

/// 统一爬虫引擎。支持单/多 Spider，共享连接池/缓存/代理池。
pub struct Engine {
    spiders: Vec<Box<dyn Spider>>,
    max_pages: usize,
    max_concurrent: Option<usize>,
    max_depth: Option<u32>,
    checkpoint_store: Option<Arc<crate::storage::Store>>,
    checkpoint_interval: usize,
    cache_store: Option<Arc<crate::storage::Store>>,
    development_mode: bool,
    proxy_pool: Option<Arc<crate::proxy::ProxyPool>>,
    request_cache: Option<RequestCache>,
}

impl Engine {
    /// 单 Spider 便捷构造。
    pub fn new(spider: impl Spider) -> Self {
        Self {
            spiders: vec![Box::new(spider)],
            max_pages: 1000,
            max_concurrent: None,
            max_depth: None,
            checkpoint_store: None,
            checkpoint_interval: 100,
            cache_store: None,
            development_mode: false,
            proxy_pool: None,
            request_cache: None,
        }
    }

    /// 多 Spider 构造。
    pub fn spiders(spiders: Vec<Box<dyn Spider>>) -> Self {
        Self {
            spiders,
            max_pages: 1000,
            max_concurrent: None,
            max_depth: None,
            checkpoint_store: None,
            checkpoint_interval: 100,
            cache_store: None,
            development_mode: false,
            proxy_pool: None,
            request_cache: None,
        }
    }

    pub fn builder(spider: impl Spider) -> Self { Self::new(spider) }
    pub fn max_pages(mut self, n: usize) -> Self { self.max_pages = n; self }
    pub fn max_concurrent(mut self, n: usize) -> Self { self.max_concurrent = Some(n); self }
    pub fn max_depth(mut self, n: u32) -> Self { self.max_depth = Some(n); self }

    pub fn with_checkpoint(mut self, store: Arc<crate::storage::Store>) -> Self {
        self.checkpoint_store = Some(store); self
    }
    pub fn checkpoint_interval(mut self, n: usize) -> Self {
        self.checkpoint_interval = n; self
    }
    pub fn development_mode(mut self, store: Arc<crate::storage::Store>) -> Self {
        self.cache_store = Some(store); self.development_mode = true; self
    }
    pub fn proxy_pool(mut self, proxies: Vec<String>, strategy: crate::proxy::RotationStrategy) -> Self {
        if !proxies.is_empty() {
            self.proxy_pool = Some(Arc::new(crate::proxy::ProxyPool::new(proxies, strategy)));
        }
        self
    }
    pub fn checkpoint(mut self, store: Arc<crate::storage::Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(store); self.checkpoint_interval = interval; self
    }
    pub fn request_cache(mut self, cache: RequestCache) -> Self {
        self.request_cache = Some(cache); self
    }

    /// 运行所有 Spider。有 schedule() 的按 cron 循环，无的执行一次。
    /// 单 Spider 时返回包含一个元素的 Vec。
    pub async fn run(self) -> Result<Vec<CrawlStats>> {
        self.run_with_sender(None).await
    }

    /// 单 Spider 便捷运行，直接返回 CrawlStats。
    pub async fn run_one(self) -> Result<CrawlStats> {
        let mut results = self.run_with_sender(None).await?;
        Ok(results.pop().unwrap_or_default())
    }

    /// 流式运行：边爬边产出事件（仅单 Spider 模式）。
    pub fn stream(self) -> CrawlStream {
        let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
        let driver = async move {
            let results = self.run_with_sender(Some(tx.clone())).await;
            match results {
                Ok(mut stats) => {
                    let s = stats.pop().unwrap_or_default();
                    let _ = tx.send(CrawlEvent::Done(s)).await;
                }
                Err(e) => {
                    let _ = tx.send(CrawlEvent::Error { url: "*".into(), error: e.to_string() }).await;
                    let _ = tx.send(CrawlEvent::Done(CrawlStats::default())).await;
                }
            }
        };
        let driver = Box::pin(driver);
        let rx = tokio_stream::wrappers::ReceiverStream::new(rx);
        use futures::StreamExt;
        let s = futures::stream::unfold(
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

    /// 内部运行逻辑：对每个 Spider 启动独立 task，共享连接池/代理池。
    async fn run_with_sender(self, tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>) -> Result<Vec<CrawlStats>> {
        // 构建共享 HTTP 客户端（用第一个 spider 的 fetcher_config，或默认）
        let fetcher_config = self.spiders.first()
            .map(|s| s.fetcher_config())
            .unwrap_or_default();
        let client = Arc::new(
            Client::builder()
                .timeout(fetcher_config.timeout)
                .build()?
        );

        let mut futures_list: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = Result<CrawlStats>>>>> = Vec::new();
        for spider in self.spiders {
            let client = client.clone();
            let proxy_pool = self.proxy_pool.clone();
            let cache_store = self.cache_store.clone();
            let checkpoint_store = self.checkpoint_store.clone();
            let tx = tx.clone();
            let max_pages = self.max_pages;
            let max_concurrent = self.max_concurrent.unwrap_or(spider.concurrent_requests() as usize);
            let max_depth = self.max_depth.unwrap_or(spider.max_depth());
            let checkpoint_interval = self.checkpoint_interval;
            let dev_mode = self.development_mode;
            let request_cache = self.request_cache.clone();

            match spider.schedule() {
                None => {
                    // 立即执行一次
                    let spider: Arc<dyn Spider> = Arc::from(spider);
                    futures_list.push(Box::pin(async move {
                        run_spider_once(
                            spider, client, proxy_pool, cache_store, checkpoint_store,
                            tx, max_pages, max_concurrent, max_depth, checkpoint_interval, dev_mode,
                            request_cache,
                        ).await
                    }));
                }
                Some(expr) => {
                    // Cron 定时循环
                    let cron = cron::CronExpr::parse(expr)?;
                    let spider: Arc<dyn Spider> = Arc::from(spider);
                    futures_list.push(Box::pin(async move {
                        loop {
                            cron.wait_until_next().await;
                            if control::is_shutdown() { break; }
                            let _ = run_spider_once(
                                spider.clone(), client.clone(), proxy_pool.clone(),
                                cache_store.clone(), checkpoint_store.clone(),
                                tx.clone(), max_pages, max_concurrent, max_depth,
                                checkpoint_interval, dev_mode, request_cache.clone(),
                            ).await;
                        }
                        Ok(CrawlStats::default())
                    }));
                }
            }
        }

        let results = futures::future::join_all(futures_list).await;
        Ok(results.into_iter().filter_map(|s| s.ok()).collect())
    }
}

/// 运行单个 Spider 一轮爬取。
#[allow(clippy::too_many_arguments)]
async fn run_spider_once(
    spider: Arc<dyn Spider>,
    client: Arc<Client>,
    proxy_pool: Option<Arc<crate::proxy::ProxyPool>>,
    cache_store: Option<Arc<crate::storage::Store>>,
    checkpoint_store: Option<Arc<crate::storage::Store>>,
    tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    max_pages: usize,
    max_concurrent: usize,
    max_depth: u32,
    checkpoint_interval: usize,
    dev_mode: bool,
    request_cache: Option<RequestCache>,
) -> Result<CrawlStats> {
    let start = std::time::Instant::now();
    let spider_name = spider.name().to_string();
    let fetcher_config = spider.fetcher_config();

    // 构建 per-spider rule engine
    let mut rule_engine = auto::ModeRuleEngine::new();
    for (pattern, mode) in spider.auto_rules() {
        let _ = rule_engine.add_user_rule(&pattern, mode);
    }
    let auto_exclude = spider.auto_exclude();

    // checkpoint 恢复
    let mut restored_state: Option<CrawlState> = None;
    if let Some(ref store) = checkpoint_store {
        if let Some(blob) = store.load_checkpoint(&spider_name)? {
            match bincode::deserialize::<CrawlState>(&blob) {
                Ok(state) => { restored_state = Some(state); }
                Err(e) => { tracing::warn!("checkpoint 反序列化失败: {}", e); }
            }
        }
    }

    let start_urls = spider.start_urls();
    let allowed = spider.allowed_domains();
    let obey_robots = spider.obey_robots();
    let fetch_mode = spider.fetch_mode();
    spider.on_start().await;

    let sched = Arc::new(scheduler::Scheduler::new());
    let robots_cache = Arc::new(Mutex::new(robots::RobotsCache::new()));

    if let Some(ref state) = restored_state {
        for req in &state.pending_urls { sched.push(req.clone()).await; }
    } else {
        for url in start_urls { sched.push(SpiderRequest::get(&url)).await; }
    }

    let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();

    let ctx = Arc::new(engine::EngineContext {
        spider: spider.clone(),
        client,
        sched: sched.clone(),
        robots_cache,
        follow_tx: follow_tx.clone(),
        follow_rx: Arc::new(Mutex::new(follow_rx)),
        stats_items: Arc::new(AtomicUsize::new(0)),
        stats_pages: Arc::new(AtomicUsize::new(0)),
        stats_errors: Arc::new(AtomicUsize::new(0)),
        stats_blocked: Arc::new(AtomicUsize::new(0)),
        stats_retries: Arc::new(AtomicUsize::new(0)),
        stats_offsite: Arc::new(AtomicUsize::new(0)),
        stats_cache_hits: Arc::new(AtomicUsize::new(0)),
        stats_status_codes: Arc::new(Mutex::new(HashMap::new())),
        domain_sems: Arc::new(Mutex::new(HashMap::new())),
        in_flight: Arc::new(AtomicUsize::new(0)),
        allowed: Arc::new(allowed),
        proxy_pool,
        rule_engine: Arc::new(Mutex::new(rule_engine)),
        auto_exclude,
        cache_store,
        fetcher_config,
        fetch_mode,
        max_pages,
        max_concurrent,
        max_depth,
        obey_robots,
        dev_mode,
        request_cache,
        abort_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        start,
        tx,
    });

    // 构建并发流
    let stream = {
        let ctx = ctx.clone();
        stream::unfold((), move |_| {
            let ctx = ctx.clone();
            async move {
                loop {
                    // 全局停止检查
                    if control::is_shutdown() || ctx.abort_flag.load(Ordering::SeqCst) {
                        return None;
                    }

                    let mut rx_guard = ctx.follow_rx.lock().await;
                    while let Ok(req) = rx_guard.try_recv() {
                        ctx.sched.push(req).await;
                    }
                    drop(rx_guard);

                    if ctx.stats_pages.load(Ordering::SeqCst) + ctx.in_flight.load(Ordering::SeqCst) >= max_pages {
                        if ctx.in_flight.load(Ordering::SeqCst) == 0 { return None; }
                        tokio::task::yield_now().await;
                        continue;
                    }

                    let req = match ctx.sched.pop().await {
                        Some(req) => req,
                        None => {
                            if ctx.in_flight.load(Ordering::SeqCst) == 0 { return None; }
                            tokio::task::yield_now().await;
                            continue;
                        }
                    };

                    ctx.in_flight.fetch_add(1, Ordering::SeqCst);
                    let ctx_c = ctx.clone();
                    let fut = async move {
                        let _guard = engine::InFlightGuard { counter: ctx_c.in_flight.clone() };
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
        if pages_since_checkpoint >= checkpoint_interval {
            if let Some(ref store) = checkpoint_store {
                engine::save_checkpoint(store, &spider_name, &sched, &ctx).await;
            }
            pages_since_checkpoint = 0;
        }
    }

    spider.on_close().await;

    if let Some(ref store) = checkpoint_store {
        if let Err(e) = store.delete_checkpoint(&spider_name) {
            tracing::warn!("删除 checkpoint 失败: {}", e);
        }
    }

    let final_status_codes = ctx.stats_status_codes.lock().await.clone();
    Ok(engine::snapshot_stats(&ctx, final_status_codes))
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
        let engine = Engine::new(CountSpider { start_url: base }).max_pages(1);
        let mut stream = engine.stream().events();
        use futures::StreamExt;
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
        let engine = Engine::new(OneSpider { start_url: base }).max_pages(1);
        let mut items_stream = engine.stream().items();
        use futures::StreamExt;
        let mut count = 0;
        while items_stream.next().await.is_some() { count += 1; }
        assert!(count >= 1, "items() 应产出至少 1 个 item");
    }
}
