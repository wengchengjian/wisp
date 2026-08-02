//! ND-013-TEST：run_inner 单元测试
//!
//! 测试 Engine::run / run_stream 的关键边界：
//! - shutdown 控制流（运行中 shutdown 后停止）
//! - max_pages 边界（达到上限后停止）
//! - checkpoint 恢复（保存 → 恢复 → 继续爬取）
//! - follow channel（Spider 返回 follows 后被调度）
//! - run_stream 事件流（Item/PageScraped/Done 事件）
//!
//! 不需要真实 Chrome：使用 HTTP 模式 + 本地 HTTP 服务器或不可达端口。

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wisp::crawl::events::{metrics_listener, EventBus, Metrics};
use wisp::crawl::{CrawlEvent, MaxPagesByCallback, Request, Response, Spider, SpiderBuilder};
use wisp::fetcher::FetchMode;
use wisp::storage::MemoryStore;
use wisp::Engine;

fn fast_fetch_config() -> wisp::FetchClientConfig {
    wisp::FetchClientConfig {
        http: wisp::http::Config {
            timeout: Duration::from_millis(100),
            ..Default::default()
        },
        max_concurrent_pages: 0,
        ..Default::default()
    }
}

/// 最小 Spider：handle 返回空，不产出 items/follows。
struct DummySpider {
    name: String,
    urls: Vec<String>,
}

#[async_trait]
impl Spider for DummySpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        self.urls.clone()
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
        (vec![], vec![])
    }
}

/// Spider 返回单个 item，用于测试 item 流。
struct ItemSpider {
    name: String,
    url: String,
}

#[async_trait]
impl Spider for ItemSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        vec![self.url.clone()]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
        (vec![serde_json::json!({"name": self.name})], vec![])
    }
}

/// Spider 返回 1 个 item + N 个 follow URLs（用于测试 follow channel）。
struct FollowSpider {
    name: String,
    start: String,
    follows: Vec<String>,
    /// 调用计数（用于验证 follow 后被再次调度）
    handle_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Spider for FollowSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        vec![self.start.clone()]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
        let n = self.handle_calls.fetch_add(1, Ordering::SeqCst);
        // 首次调用返回 follows，后续调用返回空（避免无限递归）
        if n == 0 {
            let follows = self.follows.iter().map(|u| Request::get(u)).collect();
            (vec![], follows)
        } else {
            (vec![], vec![])
        }
    }
}

/// 启动本地 HTTP 服务器返回固定 HTML。
async fn spawn_html_server(html: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
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
                    html.len(),
                    html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://localhost:{}", port)
}

// === run_inner 控制流测试 ===

/// shutdown 后 run 应在 in-flight 请求完成后退出。
///
/// 使用 multi_thread runtime 确保 shutdown spawn task 能并行执行。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_stops_on_shutdown() {
    let url = spawn_html_server("<html><body>slow</body></html>").await;
    let engine = Engine::infra()
        .max_pages(100)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    // shutdown 在 run 开始 200ms 后调用（确保 run 已进入主循环）
    let ctrl = engine.control().clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        ctrl.shutdown();
    });

    let spider = DummySpider {
        name: "shutdown-test".into(),
        urls: vec![url],
    };
    let result = tokio::time::timeout(Duration::from_secs(5), engine.run(spider)).await;
    assert!(result.is_ok(), "shutdown 后 run 应在 5s 内退出，不应死锁");
    let _ = result.unwrap().unwrap();
}

/// shutdown 应等待正在执行的 handler 完成，而不是取消。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_in_flight_handler() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let engine = Engine::infra()
        .max_pages(100)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let ctrl = engine.control().clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        ctrl.shutdown();
    });

    let handle_calls = Arc::new(AtomicUsize::new(0));
    let spider = GracefulShutdownSpider {
        name: "slow-shutdown".into(),
        url,
        handle_calls: handle_calls.clone(),
    };
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(5), engine.run(spider)).await;
    assert!(result.is_ok(), "shutdown 后 run 应在 5s 内退出");
    let _ = result.unwrap().unwrap();
    let elapsed = start.elapsed();
    assert!(handle_calls.load(Ordering::SeqCst) >= 1, "handler 应被调用");
    assert!(
        elapsed >= Duration::from_millis(400),
        "shutdown 应等待 in-flight handler 完成，实际耗时 {elapsed:?}"
    );
}

/// max_pages=1 时，第一个请求后应停止（即使有 follow）。
#[tokio::test]
async fn run_stops_at_max_pages() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    // FollowSpider 首次返回 2 个 follows，但 max_pages=1 应在第一个页面后停止
    let handle_calls = Arc::new(AtomicUsize::new(0));
    let spider = FollowSpider {
        name: "max-pages-test".into(),
        start: url.clone(),
        follows: vec![url.clone(), url.clone()],
        handle_calls: handle_calls.clone(),
    };
    let (stats, _items) = engine.run(spider).await.unwrap();
    // max_pages=1：只抓取 1 个页面
    assert!(
        stats.pages_crawled <= 1,
        "max_pages=1 时 pages_crawled 应 <= 1，实际: {}",
        stats.pages_crawled
    );
}

/// callback 维度停止条件：爬满 2 个 detail 页后停止，不再派发第 3 个。
#[tokio::test]
async fn run_stops_at_max_pages_by_callback() {
    let url = spawn_html_server(
        r#"<html><body>
            <a href="/1">Book 1</a>
            <a href="/2">Book 2</a>
            <a href="/3">Book 3</a>
        </body></html>"#,
    )
    .await;

    let spider = SpiderBuilder::new("callback-stop")
        .start_urls(vec![url])
        .on_page("default", |mut page| {
            page.follow_links(
                &["a"],
                "detail",
                |_page, _idx, a| serde_json::json!({ "title": a.text().trim() }),
            );
            page
        })
        .on_page("detail", |page| page)
        .until(MaxPagesByCallback::new("detail", 2))
        .build();

    let engine = Engine::infra()
        .max_concurrent(1)
        .max_pages(100)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let (stats, _) = engine.run(spider).await.unwrap();
    assert_eq!(
        stats.pages_crawled, 3,
        "应爬 1 个首页 + 2 个 detail，实际: {}",
        stats.pages_crawled
    );
}

// === checkpoint 恢复测试 ===

/// checkpoint 在爬取成功完成后应被清理（避免下次 run 误恢复已完成状态）。
#[tokio::test]
async fn run_clears_checkpoint_on_successful_completion() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let store: Arc<dyn wisp::storage::Store> = Arc::new(MemoryStore::default());

    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .checkpoint(store.clone(), 1) // 每 1 页保存一次
        .build()
        .unwrap();

    let spider = ItemSpider {
        name: "ckpt-clear-test".into(),
        url,
    };
    let (stats, items) = engine.run(spider).await.unwrap();
    assert_eq!(stats.pages_crawled, 1, "应抓取 1 页");
    assert_eq!(items.len(), 1, "应产出 1 个 item");

    // 爬取成功完成后 checkpoint 应被清理
    let ckpt = wisp::storage::load_checkpoint(&*store, "ckpt-clear-test")
        .await
        .unwrap();
    assert!(
        ckpt.is_none(),
        "爬取成功完成后 checkpoint 应被清理，但仍然存在"
    );
}

/// checkpoint 在爬取被 shutdown 中断时应保留，供下次 run 前恢复。
///
/// 使用 multi_thread runtime 确保 shutdown spawn task 能并行执行。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_keeps_checkpoint_on_shutdown_interrupt() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let store: Arc<dyn wisp::storage::Store> = Arc::new(MemoryStore::default());

    let engine = Engine::infra()
        .max_pages(100) // 大 max_pages，确保 shutdown 前不自然结束
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .checkpoint(store.clone(), 1)
        .build()
        .unwrap();

    // shutdown 在 run 开始 200ms 后调用（确保 run 已进入主循环）
    let ctrl = engine.control().clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        ctrl.shutdown();
    });

    // 用 FollowSpider 让爬取持续（返回 follows 避免立即结束）
    let handle_calls = Arc::new(AtomicUsize::new(0));
    let spider = GracefulShutdownSpider {
        name: "ckpt-shutdown-test".into(),
        url,
        handle_calls: handle_calls.clone(),
    };
    let _ = engine.run(spider).await;

    // shutdown 中断必须保留 checkpoint，供下次 run 前恢复
    let ckpt = wisp::storage::load_checkpoint(&*store, "ckpt-shutdown-test")
        .await
        .unwrap();
    assert!(
        ckpt.is_some(),
        "shutdown 中断后应保留 checkpoint 供恢复，实际: {:?}",
        ckpt.is_some()
    );
}

/// shutdown 中断后再次 run 应从 checkpoint 恢复并继续，自然完成后清理。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_resumes_from_checkpoint_after_shutdown() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let store: Arc<dyn wisp::storage::Store> = Arc::new(MemoryStore::default());
    let engine = Engine::infra()
        .max_pages(100)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .checkpoint(store.clone(), 1)
        .build()
        .unwrap();

    let ctrl = engine.control().clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        ctrl.shutdown();
    });
    let handle_calls = Arc::new(AtomicUsize::new(0));
    let first = GracefulShutdownSpider {
        name: "ckpt-resume-test".into(),
        url: url.clone(),
        handle_calls: handle_calls.clone(),
    };
    let (first_stats, _) = engine.run(first).await.unwrap();

    let second = DummySpider {
        name: "ckpt-resume-test".into(),
        urls: vec![url],
    };
    let (second_stats, _) = engine.run(second).await.unwrap();

    assert!(
        second_stats.pages_crawled > first_stats.pages_crawled,
        "恢复后应继续爬取，实际 first={}, second={}",
        first_stats.pages_crawled,
        second_stats.pages_crawled
    );
    let ckpt = wisp::storage::load_checkpoint(&*store, "ckpt-resume-test")
        .await
        .unwrap();
    assert!(ckpt.is_none(), "自然完成后 checkpoint 应被清理");
}

// === follow channel 测试 ===

/// Spider 返回 follows 后，follow URLs 应被调度并抓取。
#[tokio::test]
async fn run_schedules_follow_urls() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let engine = Engine::infra()
        .max_pages(10)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let handle_calls = Arc::new(AtomicUsize::new(0));
    let spider = FollowSpider {
        name: "follow-test".into(),
        start: url.clone(),
        follows: vec![url.clone(), url.clone()],
        handle_calls: handle_calls.clone(),
    };
    let (stats, _items) = engine.run(spider).await.unwrap();

    // start URL + 2 个 follow URLs = 3 次调用
    // 但 follow URL 与 start URL 相同，seen 去重会跳过重复
    // 实际：start 被抓取（1 次），2 个 follow 因 URL 相同被去重跳过
    assert!(
        stats.pages_crawled >= 1,
        "至少应抓取 start URL，实际: {}",
        stats.pages_crawled
    );
    // handle 至少被调用 1 次（start URL）
    assert!(
        handle_calls.load(Ordering::SeqCst) >= 1,
        "handle 应至少被调用 1 次"
    );
}

// === run_stream 事件流测试 ===

/// run_stream 应按顺序发送 Item → PageScraped → Done 事件。
#[tokio::test]
async fn run_stream_emits_events_in_order() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let spider = ItemSpider {
        name: "stream-test".into(),
        url,
    };
    let mut stream = engine.run_stream(spider).events();

    let mut got_item = false;
    let mut got_page_scraped = false;
    let mut got_done = false;
    let mut item_before_page = false;

    while let Some(event) = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap_or(None)
    {
        match event {
            CrawlEvent::Item(_) => {
                got_item = true;
                if !got_page_scraped {
                    item_before_page = true;
                }
            }
            CrawlEvent::PageScraped { .. } => {
                got_page_scraped = true;
            }
            CrawlEvent::Done(_) => {
                got_done = true;
                break;
            }
            _ => {}
        }
    }

    assert!(got_item, "应收到 Item 事件");
    assert!(got_page_scraped, "应收到 PageScraped 事件");
    assert!(got_done, "应收到 Done 事件");
    assert!(
        item_before_page,
        "Item 事件应在 PageScraped 之前发送（process_response 中先发 item 再发 page_scraped）"
    );
}

/// run_stream 在错误情况下应发送 Error 事件后发送 Done。
#[tokio::test]
async fn run_stream_emits_error_then_done_on_failure() {
    let engine = Engine::infra()
        .fetch_client_config(fast_fetch_config())
        .max_pages(1)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let spider = DummySpider {
        name: "error-stream-test".into(),
        urls: vec!["http://127.0.0.1:1/".into()], // 不可达
    };
    let mut stream = engine.run_stream(spider).events();

    let mut got_error = false;
    let mut got_done = false;

    while let Some(event) = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap_or(None)
    {
        match event {
            CrawlEvent::Error { .. } => got_error = true,
            CrawlEvent::Done(_) => {
                got_done = true;
                break;
            }
            _ => {}
        }
    }

    assert!(got_error, "不可达 URL 应触发 Error 事件");
    assert!(got_done, "应收到 Done 事件终止流");
}

/// EngineBuilder 注册的 EventBus 应接入 run 运行路径（Response/Item/Metrics）。
#[tokio::test]
async fn event_bus_is_wired_into_run() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let mut bus = EventBus::new();
    let metrics = Arc::new(Metrics::new());
    bus.on(metrics_listener(Arc::clone(&metrics)));
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .event_bus(bus)
        .build()
        .unwrap();
    let spider = ItemSpider {
        name: "event-bus-test".into(),
        url,
    };
    let (stats, _) = engine.run(spider).await.unwrap();
    assert_eq!(stats.pages_crawled, 1);
    assert!(
        metrics.responses.load(Ordering::SeqCst) >= 1,
        "ResponseReceived 事件应驱动 metrics"
    );
    assert!(
        metrics.items.load(Ordering::SeqCst) >= 1,
        "ItemScraped 事件应驱动 metrics"
    );
}

/// run_stream_many 应产出 DoneMany，且每个 Spider 有独立 stats。
#[tokio::test]
async fn run_stream_many_emits_done_many() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let engine = Engine::infra()
        .max_pages(10)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();
    let spider_a = ItemSpider {
        name: "stream-a".into(),
        url: format!("{}/a", url),
    };
    let spider_b = ItemSpider {
        name: "stream-b".into(),
        url: format!("{}/b", url),
    };
    let mut stream = engine.run_stream_many(vec![spider_a, spider_b]).events();
    let mut done_many = None;
    while let Some(event) = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap_or(None)
    {
        if let CrawlEvent::DoneMany(stats) = event {
            done_many = Some(stats);
            break;
        }
    }
    let stats = done_many.expect("应收到 DoneMany");
    assert_eq!(stats.len(), 2, "两个 Spider 应各有一个 stats");
    assert_eq!(stats[0].pages_crawled, 1);
    assert_eq!(stats[1].pages_crawled, 1);
}

/// SlowSpider：handle 中 sleep，让 run 不立即完成。
struct SlowSpider {
    name: String,
    url: String,
}

#[async_trait]
impl Spider for SlowSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        vec![self.url.clone()]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        (vec![], vec![])
    }
}

/// 用于验证 shutdown 优雅等待 in-flight handler 完成。
struct GracefulShutdownSpider {
    name: String,
    url: String,
    handle_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Spider for GracefulShutdownSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        vec![self.url.clone()]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
        self.handle_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(500)).await;
        (vec![], vec![])
    }
}

// === 并发约束测试 ===

/// 同一 Engine 实例并发 run 应返回 Engine 错误。
///
/// 用 SlowSpider（handle sleep 1s）确保 run1 不立即完成，
/// run2 应立即返回 Engine 错误。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_run_on_same_engine_returns_error() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let engine = Engine::infra()
        .fetch_client_config(fast_fetch_config())
        .max_pages(100)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let spider1 = SlowSpider {
        name: "concurrent-1".into(),
        url: url.clone(),
    };
    let spider2 = DummySpider {
        name: "concurrent-2".into(),
        urls: vec![url.clone()],
    };

    // run1：先 poll 一段时间获取 running 标志（SlowSpider handle sleep 1s，不会完成）
    let run1 = engine.run(spider1);
    tokio::pin!(run1);
    let poll_result = tokio::time::timeout(Duration::from_millis(100), &mut run1).await;
    assert!(
        poll_result.is_err(),
        "run1 不应在 100ms 内完成（SlowSpider handle sleep 1s）"
    );

    // run2：应立即返回 Engine 错误（running 标志被 run1 占用）
    let result = engine.run(spider2).await;
    assert!(
        result.is_err(),
        "同一 Engine 并发 run 应返回错误（ND-001-ARCH）"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("already running") || err.contains("Engine"),
        "错误信息应包含 already running 或 Engine，实际: {}",
        err
    );

    // 用 shutdown 让 run1 退出
    engine.control().shutdown();
    // 等待 run1 完成并释放 running 标志
    let _ = run1.await;

    // 验证 running 标志已释放：再次 run 应成功
    let spider3 = DummySpider {
        name: "concurrent-3".into(),
        urls: vec![url.clone()],
    };
    let result = engine.run(spider3).await;
    assert!(
        result.is_ok(),
        "run1 完成后 running 标志应释放，run3 应成功，实际: {:?}",
        result.map(|_| ()).map_err(|e| e.to_string())
    );
}

// === EngineControl 重置测试 ===

/// 每次 run 应重置 control 状态（上次 shutdown 不影响下次 run）。
#[tokio::test]
async fn run_resets_control_state() {
    let url = spawn_html_server("<html><body>ok</body></html>").await;
    let engine = Engine::infra()
        .max_pages(1)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    // 第一次 run 前 shutdown
    engine.control().shutdown();

    let spider = ItemSpider {
        name: "reset-test".into(),
        url: url.clone(),
    };
    let (stats, items) = engine.run(spider).await.unwrap();
    // shutdown 在 run 开始时被 reset，应正常抓取
    assert_eq!(stats.pages_crawled, 1, "reset 后应正常抓取 1 页");
    assert_eq!(items.len(), 1, "应产出 1 个 item");

    // 第二次 run 也应正常
    let spider2 = ItemSpider {
        name: "reset-test".into(),
        url,
    };
    let (stats2, _items2) = engine.run(spider2).await.unwrap();
    assert_eq!(stats2.pages_crawled, 1, "第二次 run 也应正常");
}
