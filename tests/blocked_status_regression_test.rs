//! Regression: blocked/retry semantics for HTTP status responses.
//!
//! `process_response` used to increment `pages_crawled` before response
//! middleware ran. When `BlockedRetryMiddleware` exhausted its refetch rounds,
//! the response was discarded but the page counter stayed incremented.
//! This file covers 403, 503, and 200 success without retry counters.

mod common;

use std::time::Duration;
use wisp::FetchMode;
use wisp::crawl::{Engine, SpiderBuilder};

async fn run_retry_spider(base: String) -> wisp::crawl::CrawlStats {
    let spider = SpiderBuilder::new("blocked")
        .start_urls(vec![base])
        .on("default", |_resp| async move { (vec![], vec![]) })
        .build();

    let engine = Engine::infra()
        .fetch_mode(FetchMode::Http)
        .max_pages(1)
        .obey_robots(false)
        .max_refetch_rounds(2)
        .max_retries(2)
        .download_delay(Duration::from_millis(50))
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();
    stats
}

#[tokio::test]
async fn blocked_response_should_not_increment_pages_after_refetch_exhausted() {
    let base =
        common::spawn_status_server(403, "Forbidden", "blocked", "text/plain; charset=utf-8").await;
    let stats = run_retry_spider(base).await;

    assert_eq!(
        stats.pages_crawled,
        0,
        "403 不应计入成功页: {}",
        stats.summary()
    );
    assert_eq!(stats.errors, 0, "blocked refetch 不计入网络错误");
    assert_eq!(stats.retry_count, 0, "blocked refetch 不增加 retry_count");
    assert!(
        stats.blocked_requests >= 3,
        "初始 403 加两次 refetch 应计入 blocked: {}",
        stats.summary()
    );
}

#[tokio::test]
async fn blocked_503_response_should_not_increment_pages_after_refetch_exhausted() {
    let base = common::spawn_status_server(
        503,
        "Service Unavailable",
        "blocked",
        "text/plain; charset=utf-8",
    )
    .await;
    let stats = run_retry_spider(base).await;

    assert_eq!(
        stats.pages_crawled,
        0,
        "503 不应计入成功页: {}",
        stats.summary()
    );
    assert_eq!(stats.errors, 0, "blocked refetch 不计入网络错误");
    assert_eq!(stats.retry_count, 0, "blocked refetch 不增加 retry_count");
    assert!(
        stats.blocked_requests >= 3,
        "503 应计入 blocked: {}",
        stats.summary()
    );
    assert!(
        stats.status_code_counts.get(&503).copied().unwrap_or(0) >= 3,
        "503 状态码应至少统计 3 次: {}",
        stats.summary()
    );
}

#[tokio::test]
async fn http_success_does_not_increment_retry_count() {
    let base = common::spawn_status_server(200, "OK", "ok", "text/plain; charset=utf-8").await;
    let stats = run_retry_spider(base).await;

    assert_eq!(
        stats.pages_crawled,
        1,
        "成功响应应计入 1 页: {}",
        stats.summary()
    );
    assert_eq!(stats.retry_count, 0, "成功响应不应触发重试");
}
