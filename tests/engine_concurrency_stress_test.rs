//! 真实 Engine 并发压力测试。
//!
//! 使用本地 HTTP server，避免外部网络抖动：
//! - `/slow?i=...`：延迟 `delay_ms` 后返回，用于制造并发
//! - `/fast`：立即返回，用于验证 shutdown/abort 后 Engine 仍可复用
//!
//! 测试不依赖真实浏览器，适合作为默认 nextest 套件的一部分。

use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use wisp::crawl::{Engine, SpiderBuilder};
use wisp::fetcher::FetchMode;

async fn spawn_delay_server(delay_ms: u64) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let server_active = active.clone();
    let server_max_active = max_active.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let active = server_active.clone();
            let max_active = server_max_active.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let request = String::from_utf8_lossy(&buf);
                let path = request.split_whitespace().nth(1).unwrap_or("/");

                let (status, reason, body) = if path.starts_with("/slow") {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    let body = "<html><body>slow</body></html>";
                    active.fetch_sub(1, Ordering::SeqCst);
                    (200, "OK", body)
                } else {
                    (200, "OK", "<html><body>fast</body></html>")
                };

                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });

    (
        format!("http://127.0.0.1:{}", addr.port()),
        active,
        max_active,
    )
}

fn stress_engine(max_concurrent: usize, max_pages: usize) -> Engine {
    Engine::infra()
        .max_concurrent(max_concurrent)
        .max_pages(max_pages)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .expect("build stress engine")
}

fn stress_spider(name: &str, urls: Vec<String>) -> wisp::crawl::ClosureSpider {
    SpiderBuilder::new(name)
        .start_urls(urls)
        .on("default", |_resp| async move { (vec![json!({})], vec![]) })
        .build()
}

async fn wait_until_active(active: &AtomicUsize, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if active.load(Ordering::SeqCst) > 0 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not receive the request before timeout"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn high_concurrency_completes_and_respects_limit() {
    let (base, _active, max_active) = spawn_delay_server(5).await;
    let urls: Vec<String> = (0..120).map(|i| format!("{base}/slow?i={i}")).collect();
    let engine = stress_engine(24, urls.len());
    let spider = stress_spider("high-concurrency", urls);

    let (stats, items) = tokio::time::timeout(Duration::from_secs(20), engine.run(spider))
        .await
        .expect("high concurrency run should not timeout")
        .expect("high concurrency run should succeed");

    assert_eq!(stats.pages_crawled, 120);
    assert_eq!(stats.errors, 0);
    assert_eq!(items.len(), 120);

    let observed = max_active.load(Ordering::SeqCst);
    assert!(
        observed <= 24,
        "max_concurrent=24 不应超过 24 并发，实际 {observed}"
    );
    assert!(observed >= 4, "120 个慢请求应产生明显并发，实际 {observed}");
}

#[tokio::test]
async fn same_engine_reused_under_load() {
    let (base, _active, _max_active) = spawn_delay_server(2).await;
    let engine = stress_engine(16, 1000);

    for run in 0..3 {
        let urls: Vec<String> = (0..40)
            .map(|i| format!("{base}/slow?run={run}&i={i}"))
            .collect();
        let spider = stress_spider(&format!("reuse-{run}"), urls);
        let (stats, items) = engine.run(spider).await.expect("reused run should succeed");

        assert_eq!(stats.pages_crawled, 40);
        assert_eq!(stats.errors, 0);
        assert_eq!(items.len(), 40);
    }
}

#[tokio::test]
async fn shutdown_under_load_is_graceful_and_reusable() {
    let (base, active, _max_active) = spawn_delay_server(200).await;
    let engine = stress_engine(4, 1000);
    let urls: Vec<String> = (0..100)
        .map(|i| format!("{base}/slow?shutdown={i}"))
        .collect();
    let spider = stress_spider("shutdown-load", urls);

    let run_engine = engine.clone();
    let run = tokio::spawn(async move { run_engine.run(spider).await });
    wait_until_active(&active, Duration::from_secs(10)).await;
    engine.control().shutdown();

    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("shutdown run should exit")
        .expect("run task should not panic")
        .expect("shutdown run should return Ok");
    assert_eq!(result.0.errors, 0, "graceful shutdown should not error");

    let fast_spider = stress_spider("reuse-after-shutdown", vec![format!("{base}/fast")]);
    let (stats, items) = engine
        .run(fast_spider)
        .await
        .expect("run after shutdown should succeed");
    assert_eq!(stats.pages_crawled, 1);
    assert_eq!(stats.errors, 0);
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn multi_spider_shared_queue_under_load() {
    let (base, _active, max_active) = spawn_delay_server(5).await;
    let engine = stress_engine(16, 200);
    let spider_a = stress_spider(
        "shared-a",
        (0..60).map(|i| format!("{base}/slow?a={i}")).collect(),
    );
    let spider_b = stress_spider(
        "shared-b",
        (0..60).map(|i| format!("{base}/slow?b={i}")).collect(),
    );

    let (stats, items) = tokio::time::timeout(
        Duration::from_secs(20),
        engine.run_many(vec![spider_a, spider_b]),
    )
    .await
    .expect("shared queue run should not timeout")
    .expect("shared queue run should succeed");

    let pages: usize = stats.iter().map(|s| s.pages_crawled).sum();
    let errors: usize = stats.iter().map(|s| s.errors).sum();
    assert_eq!(pages, 120);
    assert_eq!(errors, 0);
    assert_eq!(items.len(), 120);

    let observed = max_active.load(Ordering::SeqCst);
    assert!(
        observed <= 16,
        "max_concurrent=16 不应超过 16 并发，实际 {observed}"
    );
    assert!(
        observed >= 4,
        "两个 Spider 共享队列应产生明显并发，实际 {observed}"
    );
}
