//! 多 Spider 独立运行测试（Task 6 重构）。
//!
//! 原 patterns 路由已废弃，改为多次 `engine.run(spider)` 独立运行。
//! 每个 Spider 拥有独立队列/去重/stats，共享底层资源（HTTP/缓存/代理）。
//!
//! - `test_max_pages_condition`: 验证 MaxPages 停止条件的判定逻辑。
//! - `test_multiple_runs_independent_stats`: 同一 Engine 多次 run，stats 独立。
//! - `test_until_stops_one_spider_without_affecting_other`: per-spider until 隔离。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use async_trait::async_trait;
use serde_json::Value;
use wisp::crawl::{
    Engine, MaxPages, Spider, SpiderRequest, SpiderResponse,
    StopCondition, StopContext,
};

#[test]
fn test_max_pages_condition() {
    // 不实际跑爬虫，只验证 StopCondition 逻辑
    let cond = MaxPages(50);
    let ctx = StopContext { pages: 50, items: 0, errors: 0, in_flight: 0, elapsed: Duration::ZERO, queue_size: 0 };
    assert!(cond.should_stop(&ctx));
}

/// 启动一个返回固定 HTML 的本地 HTTP 服务器，返回 base URL（如 `http://127.0.0.1:PORT`）。
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

#[tokio::test]
async fn test_multiple_runs_independent_stats() {
    // 同一 Engine 多次 run 不同 Spider，各自独立 stats（替代原 patterns 路由测试）。
    let server_a = spawn_html_server("<html><body>page A</body></html>").await;
    let server_b = spawn_html_server("<html><body>page B</body></html>").await;

    struct SpiderA { url: String, parsed: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for SpiderA {
        fn name(&self) -> &str { "spider-a" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    struct SpiderB { url: String, parsed: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for SpiderB {
        fn name(&self) -> &str { "spider-b" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    let parsed_a = Arc::new(AtomicUsize::new(0));
    let parsed_b = Arc::new(AtomicUsize::new(0));
    let engine = Engine::infra().max_pages(10).build().unwrap();

    let (stats_a, _) = engine
        .run(SpiderA { url: server_a, parsed: parsed_a.clone() })
        .await
        .unwrap();
    let (stats_b, _) = engine
        .run(SpiderB { url: server_b, parsed: parsed_b.clone() })
        .await
        .unwrap();

    assert_eq!(parsed_a.load(Ordering::SeqCst), 1, "SpiderA 应爬 1 页");
    assert_eq!(parsed_b.load(Ordering::SeqCst), 1, "SpiderB 应爬 1 页");
    assert_eq!(stats_a.pages_crawled, 1, "SpiderA stats 应为 1 页");
    assert_eq!(stats_b.pages_crawled, 1, "SpiderB stats 应为 1 页");
}

#[tokio::test]
async fn test_until_stops_one_spider_without_affecting_other() {
    // StoppingSpider 声明 MaxPages(1)，parse 时 follow 一个 URL。
    // until 应阻止 StoppingSpider 后续请求派发，但不影响同 Engine 下 NormalSpider 的 run。
    let server = spawn_html_server("<html><body>page</body></html>").await;

    struct StoppingSpider { url: String }
    #[async_trait]
    impl Spider for StoppingSpider {
        fn name(&self) -> &str { "stopping" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            // follow 一个 URL，验证 until 会阻止它被处理
            (vec![], vec![SpiderRequest::get(&format!("{}/2", self.url))])
        }
        fn obey_robots(&self) -> bool { false }
        fn until(&self) -> Arc<dyn StopCondition> {
            Arc::new(MaxPages(1))
        }
    }

    struct NormalSpider { url: String, parsed: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for NormalSpider {
        fn name(&self) -> &str { "normal" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    let parsed = Arc::new(AtomicUsize::new(0));
    let engine = Engine::infra().max_pages(10).build().unwrap();

    let (stats_stopping, _) = engine
        .run(StoppingSpider { url: format!("{}/stop", server) })
        .await
        .unwrap();
    let (stats_normal, _) = engine
        .run(NormalSpider { url: format!("{}/normal", server), parsed: parsed.clone() })
        .await
        .unwrap();

    assert_eq!(stats_stopping.pages_crawled, 1, "StoppingSpider 应只爬 1 页");
    assert_eq!(stats_normal.pages_crawled, 1, "NormalSpider 应爬 1 页");
    assert_eq!(parsed.load(Ordering::SeqCst), 1, "NormalSpider parse 应被调用 1 次");
}
