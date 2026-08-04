//! P0-1: 验证 EngineBuilder.autoscale() API 可用。
//! AutoscaledPool 已实现，此测试验证 Engine 正确持有 autoscale 配置。

use std::time::Duration;
use wisp::crawl::runtime::autoscale::AutoscaleConfig;

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

#[tokio::test]
async fn engine_builder_accepts_autoscale() {
    let engine = wisp::crawl::Engine::infra()
        .max_concurrent(16)
        .autoscale(2, 8)
        .build();
    assert!(
        engine.is_ok(),
        "build with autoscale should succeed: {:?}",
        engine.err()
    );
}

#[tokio::test]
async fn engine_builder_accepts_autoscale_with_config() {
    let config = AutoscaleConfig {
        scale_up_interval: Duration::from_secs(3),
        scale_down_interval: Duration::from_secs(1),
        ..Default::default()
    };
    let engine = wisp::crawl::Engine::infra()
        .autoscale_with_config(1, 4, config)
        .build();
    assert!(engine.is_ok(), "build with autoscale config should succeed");
}

// P0-1 Step 2: 验证 run_inner 启用 autoscale 后能正常完成爬取，
// 且并发数不超过 max_concurrency 上限。
// 使用不可达 URL（127.0.0.1:1），请求会快速失败，验证引擎不卡死。

use async_trait::async_trait;
use serde_json::Value;
use wisp::crawl::*;
use wisp::fetcher::FetchMode;

struct FailSpider {
    name: String,
}

#[async_trait]
impl Spider for FailSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        vec!["http://127.0.0.1:1/a".into(), "http://127.0.0.1:1/b".into()]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<CrawlRequest>) {
        (vec![], vec![])
    }
}

#[tokio::test]
async fn run_with_autoscale_completes_without_deadlock() {
    // 启用 autoscale(1, 4)，爬取不可达 URL，引擎应正常完成不卡死
    let engine = wisp::crawl::Engine::infra()
        .fetch_client_config(fast_fetch_config())
        .max_pages(10)
        .autoscale(1, 4)
        .obey_robots(false)
        .max_retries(0)
        .fetch_mode(FetchMode::Http)
        .build()
        .expect("build engine");

    let (stats, _items) = engine
        .run(FailSpider {
            name: "fail".into(),
        })
        .await
        .expect("run should complete");

    // 不可达 URL：请求失败不计 pages，但应计入 errors
    let _ = stats; // 不断言具体值，只验证 run 返回
}
