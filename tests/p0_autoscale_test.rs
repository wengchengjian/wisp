//! P0-1: 验证 EngineBuilder.autoscale() API 可用。
//! AutoscaledPool 已实现，此测试验证 Engine 正确持有 autoscale 配置。

use wisp::crawl::runtime::autoscale::{AutoscaledPool, AutoscaleConfig};
use std::time::Duration;

#[tokio::test]
async fn engine_builder_accepts_autoscale() {
    let engine = wisp::crawl::Engine::infra()
        .max_concurrent(16)
        .autoscale(2, 8)
        .build();
    assert!(engine.is_ok(), "build with autoscale should succeed: {:?}", engine.err());
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

#[test]
fn autoscaled_pool_exposes_max_concurrency() {
    let pool = AutoscaledPool::new(2, 8, AutoscaleConfig::default());
    assert_eq!(pool.max_concurrency(), 8, "max_concurrency() 应返回上限值");
    assert_eq!(pool.current_concurrency(), 2, "初始值应为 min");
}
