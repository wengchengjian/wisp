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
pub use state::CrawlState;
pub use items::{Items, JsonlWriter};
pub use builder::{SpiderBuilder, ClosureSpider};
pub use session::{SessionManager, FetcherType};
pub use auto::{SelectorTracker, ModeRuleEngine};

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
}

impl SpiderRequest {
    pub fn get(url: &str) -> Self {
        Self { url: url.to_string(), method: Method::Get, headers: HashMap::new(), body: None, meta: Value::Null, callback: None, priority: 0 }
    }
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self { url: url.to_string(), method: Method::Post, headers: HashMap::new(), body, meta: Value::Null, callback: None, priority: 0 }
    }
    pub fn with_meta(mut self, meta: Value) -> Self { self.meta = meta; self }
    pub fn with_priority(mut self, p: i32) -> Self { self.priority = p; self }
    pub fn with_callback(mut self, cb: &str) -> Self { self.callback = Some(cb.to_string()); self }
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

    /// 从当前响应 URL 解析相对链接，创建 GET 请求。
    pub fn follow(&self, href: &str) -> Option<SpiderRequest> {
        let absolute = resolve_href(&self.url, href)?;
        Some(SpiderRequest::get(&absolute))
    }
    pub fn follow_with(&self, href: &str, callback: &str) -> Option<SpiderRequest> {
        let absolute = resolve_href(&self.url, href)?;
        Some(SpiderRequest::get(&absolute).with_callback(callback))
    }
    pub fn follow_meta(&self, href: &str, meta: Value) -> Option<SpiderRequest> {
        let absolute = resolve_href(&self.url, href)?;
        Some(SpiderRequest::get(&absolute).with_meta(meta))
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

/// Engine configuration.
pub struct EngineConfig {
    pub max_pages: usize,
    pub max_concurrent: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { max_pages: 1000, max_concurrent: 8 }
    }
}

/// The crawling engine that drives a Spider.
pub struct Engine<S: Spider> {
    spider: S,
    config: EngineConfig,
    checkpoint_store: Option<Arc<crate::storage::Store>>,
    checkpoint_interval: usize,
    cache_store: Option<Arc<crate::storage::Store>>,
    development_mode: bool,
    proxy_pool: Option<Arc<crate::proxy::ProxyPool>>,
    rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    auto_exclude: HashSet<String>,
}

impl<S: Spider> Engine<S> {
    pub fn new(spider: S) -> Self {
        let max_concurrent = spider.concurrent_requests() as usize;
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in spider.auto_rules() {
            let _ = rule_engine.add_user_rule(&pattern, mode);
        }
        let auto_exclude = spider.auto_exclude();
        Self {
            spider,
            config: EngineConfig { max_concurrent, ..Default::default() },
            checkpoint_store: None,
            checkpoint_interval: 100,
            cache_store: None,
            development_mode: false,
            proxy_pool: None,
            rule_engine: Arc::new(Mutex::new(rule_engine)),
            auto_exclude,
        }
    }

    pub fn builder(spider: S) -> Self { Self::new(spider) }
    pub fn max_pages(mut self, n: usize) -> Self { self.config.max_pages = n; self }
    pub fn max_concurrent(mut self, n: usize) -> Self { self.config.max_concurrent = n; self }

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

    pub async fn run(self) -> Result<CrawlStats> {
        self.run_with_sender(None).await
    }

    /// 流式运行：边爬边产出事件。
    pub fn stream(self) -> CrawlStream {
        let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
        let driver = async move {
            let stats = self.run_with_sender(Some(tx.clone())).await;
            match stats {
                Ok(s) => { let _ = tx.send(CrawlEvent::Done(s)).await; }
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

    /// 内部运行逻辑（委托给 engine.rs 的 process_request）。
    async fn run_with_sender(self, tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>) -> Result<CrawlStats> {
        let start = std::time::Instant::now();
        let max_pages = self.config.max_pages;
        let max_concurrent = self.config.max_concurrent;
        let checkpoint_store = self.checkpoint_store.clone();
        let checkpoint_interval = self.checkpoint_interval;
        let spider_name = self.spider.name().to_string();
        let fetcher_config = self.spider.fetcher_config();

        let client = Client::builder()
            .timeout(fetcher_config.timeout)
            .build()?;

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

        let start_urls = self.spider.start_urls();
        let allowed = self.spider.allowed_domains();
        let obey_robots = self.spider.obey_robots();
        let fetch_mode = self.spider.fetch_mode();
        self.spider.on_start().await;

        let spider: Arc<dyn Spider> = Arc::new(self.spider);
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
            client: Arc::new(client),
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
            proxy_pool: self.proxy_pool.clone(),
            rule_engine: self.rule_engine.clone(),
            auto_exclude: self.auto_exclude.clone(),
            cache_store: self.cache_store.clone(),
            fetcher_config,
            fetch_mode,
            max_pages,
            max_concurrent,
            obey_robots,
            dev_mode: self.development_mode,
            start,
            tx: tx.clone(),
        });

        // 构建并发流：每次从调度器取请求，委托 process_request 处理
        let stream = {
            let ctx = ctx.clone();
            stream::unfold((), move |_| {
                let ctx = ctx.clone();
                async move {
                    loop {
                        // 排空 follow channel 中的新请求
                        let mut rx_guard = ctx.follow_rx.lock().await;
                        while let Ok(req) = rx_guard.try_recv() {
                            ctx.sched.push(req).await;
                        }
                        drop(rx_guard);

                        if ctx.stats_pages.load(Ordering::SeqCst) >= max_pages {
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
