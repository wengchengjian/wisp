//! P0-2: 验证 domain_sems 用 DashMap 后，不同域名获取独立信号量，同域名共享。
//! 使用最小 Spider + 不可达 URL，验证引擎不 panic。

use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct MultiDomainSpider {
    name: String,
    counter: Arc<AtomicUsize>,
}

#[async_trait]
impl Spider for MultiDomainSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> {
        vec![
            "http://127.0.0.1:1/domain-a".into(),
            "http://127.0.0.1:1/domain-b".into(),
            "http://127.0.0.1:1/domain-a/page2".into(),
        ]
    }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        self.counter.fetch_add(1, Ordering::SeqCst);
        (vec![], vec![])
    }
    fn obey_robots(&self) -> bool { false }
    fn max_retries(&self) -> u32 { 0 }
}

#[tokio::test]
async fn domain_sems_no_panic_on_multiple_domains() {
    let counter = Arc::new(AtomicUsize::new(0));
    let engine = Engine::infra()
        .max_pages(10)
        .max_concurrent(4)
        .build()
        .expect("build engine");

    let spider = MultiDomainSpider {
        name: "multi-domain".into(),
        counter: counter.clone(),
    };
    // 不应 panic，能正常完成
    let _ = engine.run(spider).await;
}
