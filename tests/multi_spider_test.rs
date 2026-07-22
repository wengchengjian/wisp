//! StopCondition 终止策略单元测试 + 多 Spider E2E 测试。
//!
//! - `test_max_pages_condition`: 验证 MaxPages 停止条件的判定逻辑。
//! - `test_multi_spider_routing_by_pattern`: 验证 patterns 路由将 URL 分发到对应 Spider。
//! - `test_multi_spider_until_stops_one`: 验证 per-spider until 终止策略只停止对应 Spider。

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
async fn test_multi_spider_routing_by_pattern() {
    // 两个 Spider 各自声明 patterns，引擎应按 URL 路由到匹配的 Spider。
    let server_a = spawn_html_server("<html><body>page A</body></html>").await;
    let server_b = spawn_html_server("<html><body>page B</body></html>").await;

    struct SpiderA { url: String, parsed: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for SpiderA {
        fn name(&self) -> &str { "spider-a" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        fn patterns(&self) -> Vec<String> { vec![r"/a$".to_string()] }
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
        fn patterns(&self) -> Vec<String> { vec![r"/b$".to_string()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    let parsed_a = Arc::new(AtomicUsize::new(0));
    let parsed_b = Arc::new(AtomicUsize::new(0));
    let engine = Engine::spiders(vec![
        Box::new(SpiderA { url: format!("{}/a", server_a), parsed: parsed_a.clone() }),
        Box::new(SpiderB { url: format!("{}/b", server_b), parsed: parsed_b.clone() }),
    ]).max_pages(10);

    let results = engine.run().await.unwrap();
    assert_eq!(results.len(), 2, "应返回 2 个 Spider 的统计");
    assert_eq!(parsed_a.load(Ordering::SeqCst), 1, "SpiderA 应爬 1 页");
    assert_eq!(parsed_b.load(Ordering::SeqCst), 1, "SpiderB 应爬 1 页");
}

#[tokio::test]
async fn test_multi_spider_until_stops_one() {
    // StoppingSpider 声明 MaxPages(1)，parse 时 follow 一个仍匹配自身 patterns 的 URL。
    // 引擎应在该 Spider 爬完 1 页后因 until 阻止后续请求派发，且不影响 NormalSpider。
    let server = spawn_html_server("<html><body>page</body></html>").await;

    struct StoppingSpider { url: String }
    #[async_trait]
    impl Spider for StoppingSpider {
        fn name(&self) -> &str { "stopping" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        fn patterns(&self) -> Vec<String> { vec![r"/stop".to_string()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            // follow 一个仍然匹配 /stop 的 URL，验证 until 会阻止它被处理
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
        fn patterns(&self) -> Vec<String> { vec![r"/normal$".to_string()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    let parsed = Arc::new(AtomicUsize::new(0));
    let engine = Engine::spiders(vec![
        Box::new(StoppingSpider { url: format!("{}/stop", server) }),
        Box::new(NormalSpider { url: format!("{}/normal", server), parsed: parsed.clone() }),
    ]).max_pages(10);

    let results = engine.run().await.unwrap();
    assert_eq!(results.len(), 2, "应返回 2 个 Spider 的统计");
    assert_eq!(results[0].pages_crawled, 1, "StoppingSpider 应只爬 1 页");
    assert_eq!(results[1].pages_crawled, 1, "NormalSpider 应爬 1 页");
    assert_eq!(parsed.load(Ordering::SeqCst), 1, "NormalSpider parse 应被调用 1 次");
}
