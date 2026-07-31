//! Spider-based crawling engine.

pub mod auto;
pub mod builder;
pub mod engine;
pub mod middleware;
pub mod observability;
pub mod page;
pub mod runner;
pub mod runtime;
pub mod scheduling;

// 兼容 re-export：保持 `wisp::crawl::stop::MaxPages` 等子模块路径可用
pub use observability::events;
pub use observability::state;
pub use observability::stats;
pub use runtime::autoscale;
pub use runtime::control;
pub use runtime::items;
pub use runtime::output;
pub use runtime::robots;
pub use scheduling::scheduler;
pub use scheduling::stop;

pub use auto::ModeRuleEngine;
pub use builder::{ClosureSpider, SpiderBuilder};
pub use engine::{fetch_page, fetch_page_inner, record_status};
pub use items::{Items, JsonlWriter};
pub use page::Page;
pub use runner::{Engine, EngineBuilder, EngineConfig};
pub use state::CrawlState;
pub use stop::{
    FnStopCondition, MaxErrors, MaxItems, MaxPages, MaxPagesByCallback, NeverStop,
    StopCondition, StopContext, Timeout, pages_by_callback,
};

use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub use self::stats::SpiderStats;

// 统一类型：直接使用 fetcher 的 Request/Response/Method
pub use wisp_fetcher::{Method, Request, Response};



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
///
/// ND-031-ARCH 修复：Spider 只关心业务逻辑（解析什么、如何解析），
/// 引擎配置（fetch_mode/obey_robots/max_retries/download_delay/auto_rules）
/// 已迁移到 `EngineBuilder`，由 Engine 统一管理。
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    // Required
    /// Spider 名称（用于日志和统计）。
    fn name(&self) -> &str;
    /// 起始 URL 列表。
    fn start_urls(&self) -> Vec<String>;
    /// 请求分发入口。Engine 调用此方法处理响应，返回 (items, follows)。
    async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>);

    // Optional with defaults — 业务逻辑（保留在 Spider）
    /// 允许的域名集合（空表示不限制）。
    fn allowed_domains(&self) -> HashSet<String> {
        HashSet::new()
    }
    /// 爬取开始时的钩子。
    async fn on_start(&self) {}
    /// 爬取结束时的钩子。
    async fn on_close(&self) {}
    /// 请求失败时的钩子。
    async fn on_error(&self, _req: &Request, _err: &str) {}
    /// Item 处理钩子（可过滤/转换）。
    async fn on_item(&self, item: Value) -> Option<Value> {
        Some(item)
    }
    /// 判断响应是否被拦截（默认检查状态码）。
    fn is_blocked(&self, resp: &Response) -> bool {
        BLOCKED_STATUS_CODES.contains(&resp.status)
    }
    /// 最大爬取深度。默认无限制。
    ///
    /// 保留在 Spider：爬取深度是业务范围决策（"我要爬多深"），非引擎行为。
    fn max_depth(&self) -> u32 {
        u32::MAX
    }
    /// 每个请求执行前的异步钩子。默认返回 Proceed。
    async fn on_before_request(&self, _req: &Request) -> RequestAction {
        RequestAction::Proceed
    }

    // === 终止条件（保留） ===

    /// 终止条件。默认永不停止（由引擎 max_pages 兜底）。
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(NeverStop)
    }

    /// 该 Spider 是否接受指定 callback 的请求。
    ///
    /// 多 Spider 模式下，Engine 从共享队列取出请求后按此方法路由。
    /// 默认实现接受所有请求；`ClosureSpider` 根据注册的 handler 覆盖。
    fn accepts_callback(&self, _callback: Option<&str>) -> bool {
        true
    }
}

/// 默认阻塞状态码：401/403/407/429/444/500/502/503/504
pub const BLOCKED_STATUS_CODES: &[u16] = &[401, 403, 407, 429, 444, 500, 502, 503, 504];

/// Crawling statistics.
///
/// 由 [`crate::observability::stats::SpiderStats`] 派生的只读快照，
/// 只保留引擎实际统计的字段，不再维护与运行期计数重复的死数据。
#[derive(Debug, Clone, Default)]
pub struct CrawlStats {
    /// 已爬取 item 数。
    pub items_scraped: usize,
    /// 已爬取页面数。
    pub pages_crawled: usize,
    /// 错误数。
    pub errors: usize,
    /// 总耗时。
    pub duration: Duration,
    /// 被拦截请求数。
    pub blocked_requests: usize,
    /// 重试次数。
    pub retry_count: usize,
    /// 状态码分布。
    pub status_code_counts: HashMap<u16, usize>,
    /// 站外请求数。
    pub offsite_requests_count: usize,
    /// 缓存命中数。
    pub cache_hits: usize,
}

impl CrawlStats {
    /// 生成摘要字符串。
    pub fn summary(&self) -> String {
        format!(
            "爬取完成: {} 页 / {} items / {} 错误 / 耗时 {:?}",
            self.pages_crawled,
            self.items_scraped,
            self.errors,
            self.duration
        )
    }
}

/// 爬取过程中的事件流
#[derive(Debug, Clone)]
pub enum CrawlEvent {
    /// 爬取到的 item。
    Item(Value),
    /// 页面爬取完成。
    PageScraped {
        /// 页面 URL。
        url: String,
        /// 当前统计。
        stats: CrawlStats,
    },
    /// 错误发生。
    Error {
        /// 请求 URL。
        url: String,
        /// 错误信息。
        error: String,
    },
    /// ND-001-ERR：重试事件，让 stream 消费者感知重试发生。
    /// `attempt` 为当前重试次数（从 1 开始），`max` 为上限，`error` 为失败原因。
    Retry {
        /// 请求 URL。
        url: String,
        /// 当前重试次数。
        attempt: u32,
        /// 最大重试次数。
        max: u32,
        /// 错误信息。
        error: String,
    },
    /// 爬取完成。
    Done(CrawlStats),
}

/// 流式爬取事件流
pub struct CrawlStream {
    inner: std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>>,
}

impl CrawlStream {
    /// 过滤出 item 流。
    pub fn items(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = Value>>> {
        use futures::StreamExt;
        Box::pin(self.inner.filter_map(|e| async move {
            match e {
                CrawlEvent::Item(v) => Some(v),
                _ => None,
            }
        }))
    }
    /// 获取完整事件流。
    pub fn events(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>> {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_parser::ResponseExt;
    use wisp_core::utils::resolve_href;
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
            async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
                (vec![], vec![])
            }
        }
        let spider = DummySpider;
        let blocked_resp = Response::from_http(
            403, "http://example.com".into(), HashMap::new(), vec![],
            "text/html".into(), Request::get("http://example.com"),
        );
        assert!(spider.is_blocked(&blocked_resp));
        let ok_resp = Response::from_http(
            200, "http://example.com".into(), HashMap::new(), vec![],
            "text/html".into(), Request::get("http://example.com"),
        );
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
            ..Default::default()
        };
        let s = stats.summary();
        assert!(s.contains("5 页"), "summary 应含页数: {}", s);
        assert!(s.contains("10 items"), "summary 应含 items: {}", s);
        assert!(s.contains("1 错误"), "summary 应含错误数: {}", s);
    }

    #[test]
    fn test_crawl_stats_default() {
        let stats = CrawlStats::default();
        assert_eq!(stats.items_scraped, 0);
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
            async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
                let node = resp.parse();
                let text = node.select("p").text().join("");
                (vec![serde_json::json!({"text": text})], vec![])
            }
        }
        let engine = Engine::infra().max_pages(1).obey_robots(false).build().unwrap();
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
            async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
                (vec![serde_json::json!({"v": 1})], vec![])
            }
        }
        let engine = Engine::infra().max_pages(1).obey_robots(false).build().unwrap();
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
