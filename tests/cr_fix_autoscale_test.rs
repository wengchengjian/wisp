//! 验证 autoscaler 在池饱和（in_flight 接近/超过 current）时扩容，
//! 在池空闲（in_flight 远低于 current）时缩容。
//!
//! 修复前 run_autoscaler 决策反转：饱和度高时缩容、低时扩容，
//! 与 I/O 密集型爬虫语义相反。
use std::sync::Arc;
use std::time::Duration;

use wisp::crawl::observability::stats::SpiderStats;
use wisp::crawl::runtime::autoscale::{AutoscaleConfig, AutoscaledPool};

#[tokio::test]
async fn autoscale_scales_up_when_saturated() {
    // current=min=2，in_flight=4（饱和度 2.0 > 0.9）→ 应扩容
    let pool = AutoscaledPool::new(2, 8, AutoscaleConfig {
        sample_interval: Duration::from_millis(20),
        scale_up_interval: Duration::from_millis(10),
        ..Default::default()
    });
    let stats = Arc::new(SpiderStats::new());
    // 模拟 4 个 in_flight（饱和度 = 4/2 = 2.0）
    for _ in 0..4 {
        stats.in_flight.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    let pc = Arc::clone(&pool);
    let sc = Arc::clone(&stats);
    let h = tokio::spawn(async move { pc.run_autoscaler(sc).await; });
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.abort();

    let cur = pool.current_concurrency();
    assert!(
        cur > 2,
        "饱和时应扩容（cur 应 > 初始 2），实际 cur={}",
        cur
    );
}

#[tokio::test]
async fn autoscale_does_not_grow_when_idle() {
    // current=min=2，in_flight=0（饱和度 0 < 0.7）→ 不应扩容，保持 min
    let pool = AutoscaledPool::new(2, 8, AutoscaleConfig {
        sample_interval: Duration::from_millis(20),
        scale_down_interval: Duration::from_millis(10),
        ..Default::default()
    });
    let stats = Arc::new(SpiderStats::new());
    // in_flight 保持 0

    let pc = Arc::clone(&pool);
    let sc = Arc::clone(&stats);
    let h = tokio::spawn(async move { pc.run_autoscaler(sc).await; });
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.abort();

    let cur = pool.current_concurrency();
    assert_eq!(
        cur, 2,
        "空闲时不应扩容（保持 min=2），实际 cur={}",
        cur
    );
}
