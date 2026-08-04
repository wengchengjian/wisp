//! URL scheduler with priority queue and deduplication.
//!
//! Stage 1: changed to async + Mutex to support concurrent access
//! from buffer_unordered workers in Engine.
//!
//! CR-10: 默认使用精确 URL 去重（HashSet<String>），可选 Fingerprint 模式省内存。

mod core;
mod dedup;
mod queue;

pub use core::Scheduler;
pub use dedup::DedupStrategy;

#[cfg(test)]
mod tests {
    /// Fingerprint 模式下 checkpoint seen 往返必须保持一致。
    ///
    /// 验证 restore() 在 Fingerprint 模式下对 seen_urls() 返回的哈希字符串
    /// 直接 parse 回 u64（而非再次 fingerprint），确保 seen_fp 往返正确。
    ///
    /// 关键：必须让被测 URL “在 seen 但不在 pending”——这是真实 checkpoint
    /// 场景（URL 已爬取并 pop 出 heap，seen 状态需持久化去重）。若 pending
    /// 仍含该 URL，restore 的 pending 分支会再用 fingerprint(req.url) 补回
    /// 正确 u64，掩盖 seen 分支的 bug。
    #[tokio::test]
    async fn fingerprint_seen_roundtrip_preserves_hashes() {
        use super::*;
        use crate::CrawlRequest;
        let sched = Scheduler::with_strategy(DedupStrategy::Fingerprint);
        // push 两个 URL：进入 heap 与 seen_fp
        sched.push(CrawlRequest::get("https://example.com/a")).await;
        sched.push(CrawlRequest::get("https://example.com/b")).await;
        // pop 模拟已爬取：heap 清空，但 seen_fp 保留正确指纹
        sched.pop().await;
        sched.pop().await;

        // 快照 seen（checkpoint 持久化的就是 seen 状态）
        let seen = sched.seen_urls().await;
        assert_eq!(seen.len(), 2, "快照应含 2 个哈希字符串");

        // 此时 heap 已空，pending 为空——模拟纯 seen 往返
        let pending = sched.pending_urls().await;
        assert!(pending.is_empty(), "pop 后 pending 应为空");
        sched.import_state(pending, seen).await;

        // 再 push 同样的 URL：应被 seen 判定为已爬，不入 heap
        let before = sched.len().await;
        sched.push(CrawlRequest::get("https://example.com/a")).await;
        let after = sched.len().await;
        assert_eq!(
            before, after,
            "Fingerprint 模式下 restore 后 seen 应仍能去重，实际 before={}, after={}",
            before, after
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(64))]
        #[test]
        fn scheduler_exact_dedup_matches_unique_count(urls in proptest::collection::vec("[a-zA-Z0-9:/.?=&_-]{1,80}", 0..20)) {
            let unique: std::collections::HashSet<&String> = urls.iter().collect();
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let sched = super::Scheduler::with_strategy(super::DedupStrategy::Exact);
                for url in &urls {
                    sched.push(crate::CrawlRequest::get(url)).await;
                }
                assert_eq!(sched.len().await, unique.len());
                let mut popped = 0usize;
                while sched.pop().await.is_some() {
                    popped += 1;
                }
                assert_eq!(popped, unique.len());
                assert!(sched.is_empty().await);
            });
        }
    }
}
