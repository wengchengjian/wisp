#![cfg(all(feature = "browser", feature = "stealth"))]
//! BrowserPool 在 Dynamic/Stealth 模式下的并发、取消、shutdown/abort 测试。
//!
//! 使用本地 HTTP server，避免外部站点抖动：
//! - `/probe`：延迟 `delay_ms` 后返回，用于制造并发和可取消请求
//! - `/fast`：立即返回，用于验证取消后池仍可继续分配 page
//!
//! 运行方式：
//! ```bash
//! cargo nextest run --test browser_concurrency_cancel_test --run-ignored all
//! ```

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use wisp::crawl::{Engine, Request, Response, Spider};
use wisp::fetcher::{FetchMode, Fetcher};

struct ProbeServer {
    base: String,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

async fn spawn_probe_server(delay_ms: u64) -> ProbeServer {
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

                let (status, reason, body) = if path.starts_with("/probe") {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    let body = "<html><body>probe</body></html>";
                    active.fetch_sub(1, Ordering::SeqCst);
                    (200, "OK", body)
                } else if path.starts_with("/fast") {
                    (200, "OK", "<html><body>fast</body></html>")
                } else {
                    (404, "Not Found", "not found")
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

    ProbeServer {
        base: format!("http://127.0.0.1:{}", addr.port()),
        active,
        max_active,
    }
}

fn build_fetcher(mode: FetchMode) -> Fetcher {
    let builder = match mode {
        FetchMode::Dynamic => Fetcher::dynamic(),
        FetchMode::Stealth => Fetcher::stealth(),
        FetchMode::Http | FetchMode::Auto => panic!("browser fetcher mode required"),
    };
    builder
        .headless(true)
        .max_concurrent_pages(2)
        .human_mode(false)
        .build()
        .expect("build browser fetcher")
}

fn build_engine(mode: FetchMode) -> Engine {
    let config = wisp::FetchClientConfig {
        max_concurrent_pages: 1,
        headless: true,
        force_headed_offscreen: true,
        human_mode: false,
        http: wisp::http::Config {
            timeout: Duration::from_secs(10),
            ..Default::default()
        },
        ..Default::default()
    };
    Engine::infra()
        .fetch_client_config(config)
        .fetch_mode(mode)
        .obey_robots(false)
        .max_pages(1)
        .build()
        .expect("build engine")
}

struct StaticSpider {
    name: String,
    url: String,
}

#[async_trait]
impl Spider for StaticSpider {
    fn name(&self) -> &str {
        &self.name
    }

    fn start_urls(&self) -> Vec<String> {
        vec![self.url.clone()]
    }

    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
        (vec![], vec![])
    }
}

async fn assert_concurrent_probe_limit(fetcher: &Fetcher, server: &ProbeServer) {
    let urls: Vec<String> = (0..4)
        .map(|i| format!("{}/probe?i={}", server.base, i))
        .collect();
    let f1 = fetcher.get(&urls[0]);
    let f2 = fetcher.get(&urls[1]);
    let f3 = fetcher.get(&urls[2]);
    let f4 = fetcher.get(&urls[3]);
    let (r1, r2, r3, r4) = tokio::join!(f1, f2, f3, f4);

    for result in [r1, r2, r3, r4] {
        let resp = result.expect("browser fetch should succeed");
        assert_eq!(resp.status, 200, "probe page should return 200");
    }

    let observed = server.max_active.load(Ordering::SeqCst);
    assert!(
        observed <= 2,
        "max_concurrent_pages=2 不应超过 2 并发，实际 {observed}"
    );
    assert!(observed >= 2, "4 个请求应产生至少 2 并发，实际 {observed}");
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

async fn assert_cancel_releases_pool(fetcher: &Fetcher, server: &ProbeServer) {
    let slow_url = format!("{}/probe?cancel=1", server.base);
    let mut slow = Box::pin(fetcher.get(&slow_url));

    tokio::select! {
        _ = wait_until_active(&server.active, Duration::from_secs(10)) => {}
        result = &mut slow => {
            panic!("slow fetch should still be running, completed: {result:?}");
        }
    }

    drop(slow);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let fast_url = format!("{}/fast", server.base);
    let fast = tokio::time::timeout(Duration::from_secs(8), fetcher.get(&fast_url))
        .await
        .expect("fast fetch should not timeout")
        .expect("fast fetch should succeed");
    assert_eq!(fast.status, 200, "cancel should not leak the pool permit");
}

async fn assert_engine_abort_releases_pool(mode: FetchMode, server: &ProbeServer) {
    let engine = build_engine(mode);
    let run_engine = engine.clone();
    let slow_url = format!("{}/probe?abort=1", server.base);
    let run = tokio::spawn(async move {
        run_engine
            .run(StaticSpider {
                name: "abort".into(),
                url: slow_url,
            })
            .await
    });

    wait_until_active(&server.active, Duration::from_secs(10)).await;
    run.abort();
    let _ = run.await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    let fast_url = format!("{}/fast", server.base);
    let fast = tokio::time::timeout(
        Duration::from_secs(8),
        engine.run(StaticSpider {
            name: "fast".into(),
            url: fast_url,
        }),
    )
    .await
    .expect("fast run should not timeout")
    .expect("fast run should succeed");
    assert_eq!(fast.0.errors, 0, "abort should not leak pool resources");
}

async fn assert_engine_shutdown_waits_then_reusable(mode: FetchMode, server: &ProbeServer) {
    let engine = build_engine(mode);
    let run_engine = engine.clone();
    let slow_url = format!("{}/probe?shutdown=1", server.base);
    let run = tokio::spawn(async move {
        run_engine
            .run(StaticSpider {
                name: "shutdown".into(),
                url: slow_url,
            })
            .await
    });

    wait_until_active(&server.active, Duration::from_secs(10)).await;
    engine.control().shutdown();

    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("shutdown run should exit")
        .expect("run task should not panic")
        .expect("shutdown run should return Ok");
    assert_eq!(result.0.errors, 0, "graceful shutdown should not error");

    let fast_url = format!("{}/fast", server.base);
    let fast = tokio::time::timeout(
        Duration::from_secs(8),
        engine.run(StaticSpider {
            name: "fast".into(),
            url: fast_url,
        }),
    )
    .await
    .expect("fast run should not timeout")
    .expect("fast run should succeed");
    assert_eq!(fast.0.errors, 0, "shutdown should not break subsequent run");
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn dynamic_concurrent_fetches_respect_pool_limit() {
    let server = spawn_probe_server(500).await;
    let fetcher = build_fetcher(FetchMode::Dynamic);
    assert_concurrent_probe_limit(&fetcher, &server).await;
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn stealth_concurrent_fetches_respect_pool_limit() {
    let server = spawn_probe_server(500).await;
    let fetcher = build_fetcher(FetchMode::Stealth);
    assert_concurrent_probe_limit(&fetcher, &server).await;
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn dynamic_cancelled_fetch_releases_pool_permit() {
    let server = spawn_probe_server(3000).await;
    let fetcher = build_fetcher(FetchMode::Dynamic);
    assert_cancel_releases_pool(&fetcher, &server).await;
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn stealth_cancelled_fetch_releases_pool_permit() {
    let server = spawn_probe_server(3000).await;
    let fetcher = build_fetcher(FetchMode::Stealth);
    assert_cancel_releases_pool(&fetcher, &server).await;
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn dynamic_engine_abort_releases_pool() {
    let server = spawn_probe_server(3000).await;
    assert_engine_abort_releases_pool(FetchMode::Dynamic, &server).await;
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn stealth_engine_abort_releases_pool() {
    let server = spawn_probe_server(3000).await;
    assert_engine_abort_releases_pool(FetchMode::Stealth, &server).await;
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn dynamic_engine_shutdown_waits_then_reusable() {
    let server = spawn_probe_server(3000).await;
    assert_engine_shutdown_waits_then_reusable(FetchMode::Dynamic, &server).await;
}

#[tokio::test]
#[ignore = "需要真实 Chrome"]
async fn stealth_engine_shutdown_waits_then_reusable() {
    let server = spawn_probe_server(3000).await;
    assert_engine_shutdown_waits_then_reusable(FetchMode::Stealth, &server).await;
}
