//! P1-1a: status_codes 用 DashMap<u16, AtomicUsize> 无锁计数。

use std::sync::Arc;
use wisp::crawl::SpiderStats;

#[tokio::test]
async fn status_codes_concurrent_increment_is_correct() {
    let stats = Arc::new(SpiderStats::new());
    // 并发对同一状态码累加，验证无死锁且计数正确
    let handles: Vec<_> = (0..50)
        .map(|_| {
            let s = stats.clone();
            tokio::spawn(async move {
                for _ in 0..100 {
                    wisp::crawl::record_status(&s, 200);
                    wisp::crawl::record_status(&s, 404);
                }
            })
        })
        .collect();
    for h in handles { h.await.unwrap(); }

    let snap = stats.status_codes_snapshot();
    assert_eq!(snap.get(&200).copied(), Some(5000), "200 计数应为 50*100");
    assert_eq!(snap.get(&404).copied(), Some(5000), "404 计数应为 50*100");
    assert_eq!(snap.len(), 2, "仅 2 个状态码");
}

#[tokio::test]
async fn status_codes_snapshot_reflects_recorded_status() {
    // 加强断言：snapshot 不仅要返回非空，还要精确反映记录的状态码计数。
    // 防止 status_codes_snapshot 退化为永远返回空 HashMap。
    let stats = Arc::new(SpiderStats::new());
    assert!(stats.status_codes_snapshot().is_empty(), "fresh stats snapshot 应为空");

    wisp::crawl::record_status(&stats, 200);
    wisp::crawl::record_status(&stats, 200);
    wisp::crawl::record_status(&stats, 500);

    let snap = stats.status_codes_snapshot();
    assert_eq!(snap.len(), 2, "应含 2 个状态码");
    assert_eq!(snap.get(&200).copied(), Some(2), "200 计数应为 2");
    assert_eq!(snap.get(&500).copied(), Some(1), "500 计数应为 1");
}
