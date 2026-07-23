//! Spider-based crawling engine.

pub mod middleware;
pub mod observability;
pub mod scheduling;
pub mod runtime;
pub mod engine;
pub mod runner;
pub mod builder;
pub mod auto;

// 兼容 re-export：保持 `wisp::crawl::stop::MaxPages` 等子模块路径可用
pub use scheduling::stop;
pub use scheduling::scheduler;
pub use runtime::robots;
pub use runtime::request_cache;
pub use runtime::items;
pub use runtime::control;
pub use runtime::session_pool;
pub use runtime::autoscale;
pub use runtime::output;
pub use runtime::cache;
pub use observability::events;
pub use observability::stats;
pub use observability::state;

pub use state::CrawlState;
pub use items::{Items, JsonlWriter};
pub use builder::{SpiderBuilder, ClosureSpider};
pub use auto::{SelectorTracker, ModeRuleEngine};
pub use request_cache::RequestCache;
pub use stop::{StopCondition, StopContext, MaxPages, MaxItems, MaxErrors, Timeout, NeverStop, FnStopCondition};
pub use runner::{Engine, EngineBuilder};
pub use engine::record_status;

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
pub use self::stats::SpiderStats;

/// HTTP method for spider requests.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Method { Get, Post, Put, Delete }

impl Method {
    /// 返回标准 HTTP 动词字符串（大写）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

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
    // Task 3：必须用 `#[serde(skip)]` 而非 `#[serde(default)]`。
    // `serde_json::Value` 的 Deserialize 依赖 `deserialize_any`，bincode 1.x 不支持；
    // 用 `#[serde(default)]` 会让 `bincode::deserialize::<CrawlState>`（含 SpiderRequest）
    // 在 checkpoint 恢复路径抛 `DeserializeAnyNotSupported`，导致 seen/pending 全部丢失。
    // `#[serde(skip)]` 在序列化与反序列化两端都跳过 meta（用 Value::Null 默认值），
    // 与 Task 9 的既定行为一致（meta 当前不从 checkpoint 读回）。
    // 83cb940 误改为 `#[serde(default)]` 引入回归，此处恢复。
    #[serde(skip)]
    pub meta: Value,
    pub callback: Option<String>,
    pub priority: i32,
    /// 深度：起始 URL 为 0，每 follow 一次 +1。
    #[serde(default)]
    pub depth: u32,
    /// 代理 URL（由 ProxyInjectionMiddleware 设置，引擎读取并应用）。
    #[serde(skip)]
    pub proxy: Option<String>,
    /// 抓取模式覆盖（由 StealthUpgradeMiddleware 等设置，引擎优先使用此模式）。
    #[serde(skip)]
    pub fetch_mode_override: Option<FetchMode>,
}

impl SpiderRequest {
    pub fn get(url: &str) -> Self {
        Self { url: url.to_string(), method: Method::Get, headers: HashMap::new(), body: None, meta: Value::Null, callback: None, priority: 0, depth: 0, proxy: None, fetch_mode_override: None }
    }
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self { url: url.to_string(), method: Method::Post, headers: HashMap::new(), body, meta: Value::Null, callback: None, priority: 0, depth: 0, proxy: None, fetch_mode_override: None }
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
        let content_type = self.headers.get("content-type").map(|s| s.as_str()).unwrap_or("");
        Ok(crate::http::encoding::decode(&self.body, content_type))
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
            t.lock().unwrap_or_else(|e| e.into_inner()).record(sel, result.len());
        }
        result
    }

    /// XPath 查询（Auto 模式自动追踪）。
    pub fn xpath_auto(&self, expr: &str) -> NodeList {
        let result = self.parse().map(|doc| doc.xpath(expr)).unwrap_or_else(|_| NodeList::new(vec![]));
        if let Some(ref t) = self.tracker {
            t.lock().unwrap_or_else(|e| e.into_inner()).record(expr, result.len());
        }
        result
    }
}

fn resolve_href(base: &str, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let base_url = url::Url::parse(base).ok()?;
    let joined = base_url.join(href).ok()?;
    // 仅接受 http/https 结果（过滤 javascript: mailto: data: 等被 join 构造的非法 URL）
    if joined.scheme() == "http" || joined.scheme() == "https" {
        Some(joined.to_string())
    } else {
        None
    }
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
    fn fetch_mode(&self) -> FetchMode { FetchMode::Http }
    fn auto_rules(&self) -> Vec<(String, FetchMode)> { Vec::new() }
    fn auto_exclude(&self) -> HashSet<String> { HashSet::new() }
    /// 最大爬取深度。默认无限制。
    fn max_depth(&self) -> u32 { u32::MAX }
    /// 每个请求执行前的异步钩子。默认返回 Proceed。
    async fn on_before_request(&self, _req: &SpiderRequest) -> RequestAction {
        RequestAction::Proceed
    }

    // === 终止条件（保留） ===

    /// 终止条件。默认永不停止（由引擎 max_pages 兖底）。
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(NeverStop)
    }

    // === 中间件/管道（可选） ===

    /// 返回此 Spider 的中间件列表。默认为空。
    fn middlewares(&self) -> Vec<Arc<dyn middleware::Middleware>> { Vec::new() }
    /// 返回此 Spider 的 Item 管道列表。默认为空。
    fn pipelines(&self) -> Vec<Arc<dyn middleware::ItemPipeline>> { Vec::new() }
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

    #[test]
    fn resolve_href_rejects_non_http_schemes() {
        // 绝对 URL：仅 http/https 通过
        assert!(resolve_href("https://example.com", "https://other.com/p").is_some());
        assert!(resolve_href("https://example.com", "http://other.com/p").is_some());
        // 非 http scheme 应拒绝
        assert!(resolve_href("https://example.com", "javascript:void(0)").is_none(),
            "javascript: scheme 应被拒绝");
        assert!(resolve_href("https://example.com", "mailto:a@b.com").is_none(),
            "mailto: scheme 应被拒绝");
        assert!(resolve_href("https://example.com", "data:text/html,xxx").is_none(),
            "data: scheme 应被拒绝");
        // 相对链接仍正常解析
        assert!(resolve_href("https://example.com/a/", "b").is_some());
        assert_eq!(resolve_href("https://example.com/a/", "b"), Some("https://example.com/a/b".into()));
    }

    #[test]
    fn spider_response_css_with_tracker_does_not_panic() {
        use std::sync::{Arc, Mutex};
        use crate::crawl::auto::SelectorTracker;

        let tracker = Arc::new(Mutex::new(SelectorTracker::new()));
        let resp = SpiderResponse {
            url: "http://example.com".into(),
            status: 200,
            headers: std::collections::HashMap::new(),
            body: b"<html><body><p>x</p></body></html>".to_vec(),
            request: SpiderRequest::get("http://example.com"),
            tracker: Some(tracker),
            from_cache: false,
        };
        // 不应 panic
        let nodes = resp.css("p");
        assert_eq!(nodes.iter().count(), 1);
        // tracker 应记录（SelectorTracker.records 为私有，用 len() 方法）
        let t = resp.tracker.as_ref().unwrap().lock().unwrap();
        assert_eq!(t.len(), 1, "应记录 1 个选择器匹配");
        assert_eq!(t.records().len(), 1);
    }

    #[test]
    fn test_method_as_str_returns_standard_verbs() {
        assert_eq!(Method::Get.as_str(), "GET");
        assert_eq!(Method::Post.as_str(), "POST");
        assert_eq!(Method::Put.as_str(), "PUT");
        assert_eq!(Method::Delete.as_str(), "DELETE");
    }
}
