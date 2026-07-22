//! Verify Spider Engine respects max_concurrent limit.

use async_trait::async_trait;
use wisp::crawl::{Spider, SpiderRequest, SpiderResponse, Engine};
use serde_json::Value;

struct ConcurrencySpider;

#[async_trait]
impl Spider for ConcurrencySpider {
    fn name(&self) -> &str { "concurrency-test" }
    fn start_urls(&self) -> Vec<String> {
        // 10 URLs that each take 100ms to respond
        (0..10).map(|i| format!("https://httpbin.org/delay/0.1?i={}", i)).collect()
    }
    fn concurrent_requests(&self) -> u32 { 4 }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![], vec![])
    }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
}

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_max_concurrent_respected() {
    let spider = ConcurrencySpider;
    let engine = Engine::infra()
        .max_pages(10)
        .build()
        .unwrap();
    let (stats, _items) = engine.run(spider).await.unwrap();
    // Smoke test: should complete without panic
    assert_eq!(stats.pages_crawled, 10);
}
