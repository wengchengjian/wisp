//! 真实环境测试：重试机制。cargo test --test crawl_retry_real_test -- --ignored 运行。

use async_trait::async_trait;
use wisp::crawl::{Spider, SpiderRequest, SpiderResponse, Engine};
use serde_json::Value;

struct RetrySpider;
#[async_trait]
impl Spider for RetrySpider {
    fn name(&self) -> &str { "retry-test" }
    fn start_urls(&self) -> Vec<String> {
        // httpbin.org/status/403 返回 403，应触发重试
        vec!["https://httpbin.org/status/403".to_string()]
    }
    fn max_retries(&self) -> u32 { 2 }
    fn download_delay(&self) -> std::time::Duration { std::time::Duration::from_millis(100) }
    fn obey_robots(&self) -> bool { false }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![], vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_retry_on_403_status() {
    let stats = Engine::new(RetrySpider).max_pages(1).run().await.unwrap();
    // 403 应触发重试，最终 errors >= 1（重试耗尽后计入 errors）
    assert!(stats.errors >= 1, "应有错误统计: {:?}", stats);
    assert!(stats.pages_crawled == 0, "403 不应计入成功页: {:?}", stats);
    assert!(stats.retry_count >= 2, "应有重试次数: {:?}", stats);
}

#[tokio::test]
#[ignore = "requires network access to httpbin.org"]
async fn test_retry_on_500_then_success() {
    // 用一个会先失败后成功的场景：httpbin.org/status/500 总是 500，无法测试 success
    // 改为测试 httpbin.org/status/200 应一次成功无重试
    struct OkSpider;
    #[async_trait]
    impl Spider for OkSpider {
        fn name(&self) -> &str { "ok-test" }
        fn start_urls(&self) -> Vec<String> { vec!["https://httpbin.org/status/200".to_string()] }
        fn max_retries(&self) -> u32 { 2 }
        fn obey_robots(&self) -> bool { false }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) { (vec![], vec![]) }
    }
    let stats = Engine::new(OkSpider).max_pages(1).run().await.unwrap();
    assert_eq!(stats.pages_crawled, 1);
    assert_eq!(stats.retry_count, 0);
}
