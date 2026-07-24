//! Spider-based crawling engine.

pub mod auto;
pub mod builder;
pub mod engine;
pub mod middleware;
pub mod observability;
pub mod runner;
pub mod runtime;
pub mod scheduling;

// 兼容 re-export：保持 `wisp::crawl::stop::MaxPages` 等子模块路径可用
pub use observability::events;
pub use observability::state;
pub use observability::stats;
pub use runtime::autoscale;
pub use runtime::cache;
pub use runtime::control;
pub use runtime::items;
pub use runtime::output;
pub use runtime::request_cache;
pub use runtime::robots;
pub use runtime::session_pool;
pub use scheduling::scheduler;
pub use scheduling::stop;

pub use auto::ModeRuleEngine;
pub use builder::{ClosureSpider, SpiderBuilder};
pub use engine::{fetch_page, fetch_page_inner, record_status};
pub use items::{Items, JsonlWriter};
pub use request_cache::RequestCache;
pub use runner::{Engine, EngineBuilder};
pub use state::CrawlState;
pub use stop::{
    FnStopCondition, MaxErrors, MaxItems, MaxPages, NeverStop, StopCondition, StopContext, Timeout,
};

use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub use self::stats::SpiderStats;
use crate::fetcher::FetchMode;

// 统一类型：直接使用 fetcher 的 Request/Response/Method
pub use crate::fetcher::{Method, Request, Response};



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



/// The core Spider trait users implement to define a crawler.
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    // Required
    fn name(&self) -> &str;
    fn start_urls(&self) -> Vec<String>;
    async fn parse(&self, response: Response) -> (Vec<Value>, Vec<Request>);

    /// 请求分发入口。Engine 调用此方法（不直接调 parse）。
    async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
        self.parse(resp).await
    }

    // Optional with defaults
    fn allowed_domains(&self) -> HashSet<String> {
        HashSet::new()
    }
    fn download_delay(&self) -> Duration {
        Duration::from_millis(0)
    }
    fn obey_robots(&self) -> bool {
        true
    }
    fn max_retries(&self) -> u32 {
        3
    }
    fn fetch_client_config(&self) -> crate::fetcher::FetchClientConfig {
        crate::fetcher::FetchClientConfig::default()
    }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
    async fn on_error(&self, _req: &Request, _err: &str) {}
    async fn on_item(&self, item: Value) -> Option<Value> {
        Some(item)
    }
    fn is_blocked(&self, resp: &Response) -> bool {
        BLOCKED_STATUS_CODES.contains(&resp.status)
    }
    fn fetch_mode(&self) -> FetchMode {
        FetchMode::Http
    }
    fn auto_rules(&self) -> Vec<(String, FetchMode)> {
        Vec::new()
    }
    /// 最大爬取深度。默认无限制。
    fn max_depth(&self) -> u32 {
        u32::MAX
    }
    /// 每个请求执行前的异步钩子。默认返回 Proceed。
    async fn on_before_request(&self, _req: &Request) -> RequestAction {
        RequestAction::Proceed
    }

    // === 终止条件（保留） ===

    /// 终止条件。默认永不停止（由引擎 max_pages 兖底）。
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(NeverStop)
    }

    // === 中间件/管道（可选） ===

    /// 返回此 Spider 的中间件列表。默认为空。
    fn middlewares(&self) -> Vec<Arc<dyn middleware::Middleware>> {
        Vec::new()
    }
    /// 返回此 Spider 的 Item 管道列表。默认为空。
    fn pipelines(&self) -> Vec<Arc<dyn middleware::ItemPipeline>> {
        Vec::new()
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
            self.pages_crawled,
            self.items_scraped,
            self.errors,
            self.duration,
            self.bytes_downloaded as f64 / 1024.0,
            self.avg_response_time
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
            match e {
                CrawlEvent::Item(v) => Some(v),
                _ => None,
            }
        }))
    }
    pub fn events(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>> {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::resolve_href;
    use futures::StreamExt;
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
            fn name(&self) -> &str {
                "dummy"
            }
            fn start_urls(&self) -> Vec<String> {
                vec![]
            }
            async fn parse(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
                (vec![], vec![])
            }
        }
        let spider = DummySpider;
        let blocked_resp = Response::from_http(
            403, "http://example.com".into(), HashMap::new(), vec![],
            "text/html".into(), Request::get("http://example.com"),
        );
        assert!(spider.is_blocked(&blocked_resp));
        let ok_resp = Response { status: 200, ..blocked_resp };
        assert!(!spider.is_blocked(&ok_resp));
    }

    async fn spawn_html_server(html: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
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
            items_scraped: 10,
            pages_crawled: 5,
            errors: 1,
            duration: Duration::from_secs(30),
            bytes_downloaded: 2048,
            avg_response_time: Duration::from_millis(500),
            domain_counts: {
                let mut m = HashMap::new();
                m.insert("example.com".to_string(), 5);
                m
            },
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
        struct CountSpider {
            start_url: String,
        }
        #[async_trait]
        impl Spider for CountSpider {
            fn name(&self) -> &str {
                "count"
            }
            fn start_urls(&self) -> Vec<String> {
                vec![self.start_url.clone()]
            }
            async fn parse(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
                let node = resp.parse();
                let text = node.select("p").text().join("");
                (vec![serde_json::json!({"text": text})], vec![])
            }
            fn obey_robots(&self) -> bool {
                false
            }
        }
        let engine = Engine::infra().max_pages(1).build().unwrap();
        let mut stream = engine.run_stream(CountSpider { start_url: base }).events();
        let mut items = 0;
        let mut done = false;
        while let Some(event) = stream.next().await {
            match event {
                CrawlEvent::Item(_) => items += 1,
                CrawlEvent::Done(stats) => {
                    assert!(stats.pages_crawled >= 1);
                    done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(done, "应收到 Done 事件");
        assert!(items >= 1, "应至少收到 1 个 Item 事件, 实际 {}", items);
    }

    #[tokio::test]
    async fn test_stream_items_helper() {
        let base = spawn_html_server("<p>hello</p>").await;
        struct OneSpider {
            start_url: String,
        }
        #[async_trait]
        impl Spider for OneSpider {
            fn name(&self) -> &str {
                "one"
            }
            fn start_urls(&self) -> Vec<String> {
                vec![self.start_url.clone()]
            }
            async fn parse(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
                (vec![serde_json::json!({"v": 1})], vec![])
            }
            fn obey_robots(&self) -> bool {
                false
            }
        }
        let engine = Engine::infra().max_pages(1).build().unwrap();
        let mut items_stream = engine.run_stream(OneSpider { start_url: base }).items();
        let mut count = 0;
        while items_stream.next().await.is_some() {
            count += 1;
        }
        assert!(count >= 1, "items() 应产出至少 1 个 item");
    }

    #[test]
    fn resolve_href_rejects_non_http_schemes() {
        assert!(resolve_href("https://example.com", "https://other.com/p").is_some());
        assert!(resolve_href("https://example.com", "http://other.com/p").is_some());
        assert!(
            resolve_href("https://example.com", "javascript:void(0)").is_none(),
            "javascript: scheme 应被拒绝"
        );
        assert!(
            resolve_href("https://example.com", "mailto:a@b.com").is_none(),
            "mailto: scheme 应被拒绝"
        );
        assert!(
            resolve_href("https://example.com", "data:text/html,xxx").is_none(),
            "data: scheme 应被拒绝"
        );
        assert!(resolve_href("https://example.com/a/", "b").is_some());
        assert_eq!(
            resolve_href("https://example.com/a/", "b"),
            Some("https://example.com/a/b".into())
        );
    }

    #[test]
    fn response_css_works() {
        let resp = Response::from_http(
            200, "http://example.com".into(), HashMap::new(),
            b"<html><body><p>x</p></body></html>".to_vec(),
            "text/html; charset=utf-8".into(),
            Request::get("http://example.com"),
        );
        let nodes = resp.css("p");
        assert_eq!(nodes.iter().count(), 1);
    }

    #[test]
    fn test_method_as_str_returns_standard_verbs() {
        assert_eq!(Method::Get.as_str(), "GET");
        assert_eq!(Method::Post.as_str(), "POST");
        assert_eq!(Method::Put.as_str(), "PUT");
        assert_eq!(Method::Delete.as_str(), "DELETE");
    }
}
