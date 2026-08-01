//! Crawl engine 库测试。

use super::*;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use wisp_core::utils::resolve_href;
use wisp_parser::ResponseExt;

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
        403,
        "http://example.com".into(),
        HashMap::new(),
        vec![],
        "text/html".into(),
        Request::get("http://example.com"),
    );
    assert!(spider.is_blocked(&blocked_resp));
    let ok_resp = Response::from_http(
        200,
        "http://example.com".into(),
        HashMap::new(),
        vec![],
        "text/html".into(),
        Request::get("http://example.com"),
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
            (vec![serde_json::json!({ "text": text })], vec![])
        }
    }
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .build()
        .unwrap();
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
            (vec![serde_json::json!({ "v": 1 })], vec![])
        }
    }
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .build()
        .unwrap();
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
        200,
        "http://example.com".into(),
        HashMap::new(),
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
