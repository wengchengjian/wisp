//! 真实环境测试：响应缓存 replay 模式。
//!
//! 运行方式：`cargo test --test crawl_cache_real_test -- --ignored`
//! 需要网络访问 httpbin.org。

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use wisp::crawl::{Engine, Spider, Request, Response};
use wisp::storage::Store;

struct CacheSpider;
#[async_trait]
impl Spider for CacheSpider {
    fn name(&self) -> &str {
        "cache-test"
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["https://httpbin.org/get".to_string()]
    }
    fn obey_robots(&self) -> bool {
        false
    }
    async fn parse(&self, resp: Response) -> (Vec<Value>, Vec<Request>) {
        let text = resp.text().unwrap_or_default();
        assert!(text.contains("httpbin.org"), "响应应来自 httpbin");
        (vec![], vec![])
    }
}

#[tokio::test]
#[ignore = "requires network access"]
async fn test_development_mode_caches_response() {
    let store = Arc::new(Store::open_in_memory().unwrap());

    // 第一次运行：发网络请求，保存缓存
    let engine = Engine::infra()
        .max_pages(1)
        .build()
        .unwrap();
    let (stats1, _) = engine.run(CacheSpider).await.unwrap();
    assert_eq!(stats1.pages_crawled, 1);
    assert_eq!(stats1.cache_hits, 0, "第一次运行不应有缓存命中");

    // 验证缓存已保存
    let cached = store
        .load_cached_response("https://httpbin.org/get", "GET")
        .unwrap();
    assert!(cached.is_some(), "响应应已缓存");

    // 第二次运行：命中缓存（同一 engine 复用，新 spider 实例）
    let (stats2, _) = engine.run(CacheSpider).await.unwrap();
    assert_eq!(stats2.pages_crawled, 1);
    assert_eq!(stats2.cache_hits, 1, "第二次应命中缓存");
}
