//! 队列/去重状态快照。

use super::*;

impl Scheduler {
    /// Snapshot the pending URLs (for checkpoint).
    pub async fn pending_urls(&self) -> Vec<Request> {
        let g = self.heap.lock().await;
        // Note: BinaryHeap is max-heap, iteration order is unspecified.
        // We sort by priority to give a deterministic checkpoint.
        let mut reqs: Vec<PrioritizedRequest> = g.heap.iter().cloned().collect();
        // Need Clone bound on PrioritizedRequest - add it
        reqs.sort_by(|a, b| b.cmp(a));
        reqs.into_iter().map(|p| p.req).collect()
    }

    /// Snapshot the seen URLs (for checkpoint).
    ///
    /// Exact 模式返回真实 URL；Fingerprint 模式返回 hash 字符串。
    pub async fn seen_urls(&self) -> HashSet<String> {
        match self.strategy {
            DedupStrategy::Exact => self.seen_exact.iter().map(|s| s.clone()).collect(),
            DedupStrategy::Fingerprint => self.seen_fp.iter().map(|h| h.to_string()).collect(),
        }
    }
}
