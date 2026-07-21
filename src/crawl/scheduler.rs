//! URL scheduler with priority queue and deduplication.
//!
//! Stage 1: changed to async + Mutex to support concurrent access
//! from buffer_unordered workers in Engine.

use std::collections::{BinaryHeap, HashSet};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::sync::Arc;
use tokio::sync::Mutex;
use super::SpiderRequest;

struct PrioritizedRequest {
    req: SpiderRequest,
    seq: u64,
}

impl PartialEq for PrioritizedRequest {
    fn eq(&self, other: &Self) -> bool { self.req.priority == other.req.priority && self.seq == other.seq }
}
impl Eq for PrioritizedRequest {}
impl PartialOrd for PrioritizedRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for PrioritizedRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        self.req.priority.cmp(&other.req.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// Inner state guarded by Mutex.
struct SchedulerInner {
    heap: BinaryHeap<PrioritizedRequest>,
    seen: HashSet<u64>,
    seq: u64,
}

/// Async URL scheduler with deduplication. Cloneable for sharing across tasks.
#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<Mutex<SchedulerInner>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SchedulerInner {
                heap: BinaryHeap::new(),
                seen: HashSet::new(),
                seq: 0,
            })),
        }
    }

    /// Push a request (deduplicates by URL fingerprint).
    pub async fn push(&self, req: SpiderRequest) {
        let fp = fingerprint(&req.url);
        let mut g = self.inner.lock().await;
        if g.seen.insert(fp) {
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seq += 1;
        }
    }

    /// Pop the highest-priority request.
    pub async fn pop(&self) -> Option<SpiderRequest> {
        let mut g = self.inner.lock().await;
        g.heap.pop().map(|p| p.req)
    }

    /// Snapshot the pending URLs (for checkpoint).
    pub async fn pending_urls(&self) -> Vec<SpiderRequest> {
        let g = self.inner.lock().await;
        // Note: BinaryHeap is max-heap, iteration order is unspecified.
        // We sort by priority to give a deterministic checkpoint.
        let mut reqs: Vec<PrioritizedRequest> = g.heap.iter().cloned().collect();
        // Need Clone bound on PrioritizedRequest - add it
        reqs.sort_by(|a, b| b.cmp(a));
        reqs.into_iter().map(|p| p.req).collect()
    }

    /// Snapshot the seen URLs (for checkpoint).
    ///
    /// Stage 1 placeholder: 返回 hash 字符串而非原始 URL（Scheduler 内部
    /// 只存 u64 fingerprint）。stage 2 应改为维护 `HashSet<String>` 原始 URL。
    /// 当前 CrawlState::seen_urls 使用 `HashSet::new()` 绕过此方法。
    pub async fn seen_urls(&self) -> HashSet<String> {
        // placeholder: 返回 hash 字符串，不是真实 URL
        let g = self.inner.lock().await;
        g.seen.iter().map(|h| h.to_string()).collect()
    }

    /// Number of pending requests.
    pub async fn len(&self) -> usize {
        self.inner.lock().await.heap.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.heap.is_empty()
    }

    /// Replace inner state (for checkpoint restore).
    ///
    /// Stage 1 未启用：Task 9 用 `push()` 循环恢复 pending_urls。
    /// Stage 2 改用此方法批量恢复（含 seen_urls）。
    #[allow(dead_code)]
    pub async fn restore(&self, pending: Vec<SpiderRequest>, seen: HashSet<String>) {
        let mut g = self.inner.lock().await;
        g.heap.clear();
        g.seen.clear();
        g.seq = 0;
        // Rebuild seen as hashes of URLs
        for url in &seen {
            g.seen.insert(fingerprint(url));
        }
        // Re-queue pending (they will be deduplicated against seen)
        for req in pending {
            let fp = fingerprint(&req.url);
            // Force insert even if seen (they're already in seen set)
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seen.insert(fp);
            g.seq += 1;
        }
    }
}

// Add Clone bound for PrioritizedRequest (needed by pending_urls)
impl Clone for PrioritizedRequest {
    fn clone(&self) -> Self {
        Self { req: self.req.clone(), seq: self.seq }
    }
}

fn fingerprint(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}
