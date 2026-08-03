use std::time::Duration;

use super::*;

use crate::stats::SpiderStats;

#[test]
fn test_autoscaled_pool_creation() {
    let pool = AutoscaledPool::new(2, 16, AutoscaleConfig::default());
    assert_eq!(pool.current_concurrency(), 2);
}

#[test]
fn test_autoscaled_pool_min_max() {
    let pool = AutoscaledPool::new(0, 0, AutoscaleConfig::default());
    // min 至少为 1
    assert_eq!(pool.current_concurrency(), 1);
}

#[tokio::test]
async fn test_autoscaler_runs() {
    let pool = AutoscaledPool::new(
        2,
        8,
        AutoscaleConfig {
            sample_interval: Duration::from_millis(50),
            ..Default::default()
        },
    );
    let stats = Arc::new(SpiderStats::new());
    let pool_clone = Arc::clone(&pool);
    let stats_clone = Arc::clone(&stats);
    let handle = tokio::spawn(async move {
        pool_clone.run_autoscaler(vec![stats_clone], None).await;
    });
    // 运行一小段时间后 abort
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.abort();
    // 并发数应仍在合理范围内
    let current = pool.current_concurrency();
    assert!((2..=8).contains(&current));
}

#[tokio::test]
async fn autoscale_scales_up_when_saturated() {
    let pool = AutoscaledPool::new(
        2,
        8,
        AutoscaleConfig {
            sample_interval: Duration::from_millis(20),
            scale_up_interval: Duration::from_millis(10),
            ..Default::default()
        },
    );
    let stats = Arc::new(SpiderStats::new());
    for _ in 0..4 {
        stats
            .in_flight
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    let pc = Arc::clone(&pool);
    let sc = Arc::clone(&stats);
    let h = tokio::spawn(async move {
        pc.run_autoscaler(vec![sc], None).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.abort();

    let cur = pool.current_concurrency();
    assert!(cur > 2, "饱和时应扩容（cur 应 > 初始 2），实际 cur={cur}");
}

#[tokio::test]
async fn autoscale_does_not_grow_when_idle() {
    let pool = AutoscaledPool::new(
        2,
        8,
        AutoscaleConfig {
            sample_interval: Duration::from_millis(20),
            scale_down_interval: Duration::from_millis(10),
            ..Default::default()
        },
    );
    let stats = Arc::new(SpiderStats::new());

    let pc = Arc::clone(&pool);
    let sc = Arc::clone(&stats);
    let h = tokio::spawn(async move {
        pc.run_autoscaler(vec![sc], None).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    h.abort();

    let cur = pool.current_concurrency();
    assert_eq!(cur, 2, "空闲时不应扩容（保持 min=2），实际 cur={cur}");
}
