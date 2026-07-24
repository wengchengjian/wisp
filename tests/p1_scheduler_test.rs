//! P1-2: Scheduler seen/heap 分离，并发不死锁。

use wisp::crawl::scheduler::{Scheduler, DedupStrategy};
use wisp::crawl::Request;

#[tokio::test]
async fn scheduler_concurrent_push_pop_dedup_correct() {
    let sched = Scheduler::new();
    // 并发 push 1000 个 URL（含 50% 重复），再 pop 全部
    let pushers: Vec<_> = (0..10)
        .map(|tid| {
            let s = sched.clone();
            tokio::spawn(async move {
                for i in 0..100 {
                    // tid*100+i，偶数为重复（0,2,4.. 跨线程共享同一组 URL）
                    let url = format!("https://example.com/{}", if tid % 2 == 0 { i } else { 1000 + tid * 100 + i });
                    s.push(Request::get(&url)).await;
                }
            })
        })
        .collect();
    for h in pushers { h.await.unwrap(); }

    // pop 全部，验证无 panic、数量 = 唯一 URL 数
    let mut popped = 0;
    while sched.pop().await.is_some() {
        popped += 1;
    }
    // 5 个偶数 tid 各推 0..99（100 个，但跨偶数 tid 重复同一组 0..99）→ 去重后 100 个
    // 5 个奇数 tid 各推 1000+tid*100+i（500 个唯一）→ 500 个
    // 总计 600 个唯一
    assert_eq!(popped, 600, "去重后应剩 600 个唯一 URL");
}

#[tokio::test]
async fn scheduler_fingerprint_strategy_seen_split_works() {
    let sched = Scheduler::with_strategy(DedupStrategy::Fingerprint);
    sched.push(Request::get("https://example.com/a")).await;
    // 重复 push 同 URL 应被去重
    sched.push(Request::get("https://example.com/a")).await;
    assert_eq!(sched.len().await, 1);
    let seen = sched.seen_urls().await;
    assert_eq!(seen.len(), 1, "Fingerprint 模式 seen 应含 1 个 hash");
}
