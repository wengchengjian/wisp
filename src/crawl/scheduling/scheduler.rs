//! URL scheduler with priority queue and deduplication.
//!
//! Stage 1: changed to async + Mutex to support concurrent access
//! from buffer_unordered workers in Engine.
//!
//! CR-10: 默认使用精确 URL 去重（HashSet<String>），可选 Fingerprint 模式省内存。

use crate::crawl::Request;
use dashmap::DashSet;
use parking_lot::Mutex;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BinaryHeap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// 去重策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupStrategy {
    /// 存储原始 URL（精确，内存较大）。默认选项，对 99% 场景足够。
    Exact,
    /// u64 指纹（省内存，有碰撞风险）。适合千万级 URL 大规模爬取。
    Fingerprint,
}

struct PrioritizedRequest {
    req: Request,
    seq: u64,
}

impl PartialEq for PrioritizedRequest {
    fn eq(&self, other: &Self) -> bool {
        self.req.priority == other.req.priority && self.seq == other.seq
    }
}
impl Eq for PrioritizedRequest {}
impl PartialOrd for PrioritizedRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PrioritizedRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        self.req
            .priority
            .cmp(&other.req.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// heap 与 seq 共享一个 Mutex（push/pop 需要原子读 seq + push/pop）。
struct HeapInner {
    heap: BinaryHeap<PrioritizedRequest>,
    seq: u64,
}

/// Scheduler：seen 集合（DashSet，无锁）与 heap（独立 Mutex）分离。
///
/// push 时先查/插 seen（DashSet，无锁），命中才锁 heap 入队；
/// pop 时只锁 heap。两者不再串行于同一锁。
///
/// 注意：`heap` 用 `parking_lot::Mutex`（同步锁），临界区内禁止 `.await`。
/// heap 的 push/pop/len 等操作都是 O(log N) 同步完成，锁持有时间极短，
/// parking_lot 的 spin 比 tokio Mutex 的 yield+reschedule 快 10-100×。
/// 如需在持锁期间 await，必须改回 `tokio::sync::Mutex` 或重构。
///
/// ND-008-ARCH：`max_seen` 字段控制 seen 集合容量告警阈值。超过时 log warn，
/// 提示用户切换 Fingerprint 模式或定期重启。默认 `usize::MAX` 表示无告警。
/// 注意：此为可观测性改进，不自动淘汰（避免重复爬取）。完整 LRU 淘汰可作为后续优化。
#[derive(Clone)]
pub struct Scheduler {
    heap: Arc<Mutex<HeapInner>>,
    seen_exact: Arc<DashSet<String>>,
    seen_fp: Arc<DashSet<u64>>,
    strategy: DedupStrategy,
    /// ND-008-ARCH：seen 集合容量告警阈值。
    max_seen: usize,
    /// 已发出过告警的标记，避免重复日志刷屏（每次超过只告警一次）。
    warned: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    /// 创建默认调度器（精确去重）。
    #[must_use]
    pub fn new() -> Self {
        Self::with_strategy(DedupStrategy::Exact)
    }

    /// 使用指定去重策略创建 Scheduler。
    #[must_use]
    pub fn with_strategy(strategy: DedupStrategy) -> Self {
        Self::with_strategy_and_max_seen(strategy, usize::MAX)
    }

    /// ND-008-ARCH：创建 Scheduler 并设置 seen 集合容量告警阈值。
    ///
    /// 当 seen 集合大小超过 `max_seen` 时，记录一次 warn 日志。
    /// 默认 `usize::MAX` 表示无告警。建议大规模爬取设置为 1_000_000。
    #[must_use]
    pub fn with_strategy_and_max_seen(strategy: DedupStrategy, max_seen: usize) -> Self {
        Self {
            heap: Arc::new(Mutex::new(HeapInner {
                heap: BinaryHeap::new(),
                seq: 0,
            })),
            seen_exact: Arc::new(DashSet::new()),
            seen_fp: Arc::new(DashSet::new()),
            strategy,
            max_seen,
            warned: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Push a request (deduplicates by URL).
    pub async fn push(&self, req: Request) {
        // seen 去重（DashSet 无锁，不阻塞 pop）
        let is_new = match self.strategy {
            DedupStrategy::Exact => self.seen_exact.insert(req.url.clone()),
            DedupStrategy::Fingerprint => self.seen_fp.insert(fingerprint(&req.url)),
        };
        if is_new {
            // ND-008-ARCH：seen 集合容量告警（只告警一次，避免刷屏）
            if self.max_seen != usize::MAX {
                let current_size = match self.strategy {
                    DedupStrategy::Exact => self.seen_exact.len(),
                    DedupStrategy::Fingerprint => self.seen_fp.len(),
                };
                if current_size > self.max_seen
                    && !self.warned.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    tracing::warn!(
                        "Scheduler seen 集合已超过告警阈值 {} (当前 {})，\
                         长时间爬取可能导致内存增长。建议切换 Fingerprint 模式或定期重启。",
                        self.max_seen,
                        current_size
                    );
                }
            }
            let mut g = self.heap.lock();
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seq += 1;
        }
    }

    /// Pop the highest-priority request.
    pub async fn pop(&self) -> Option<Request> {
        let mut g = self.heap.lock();
        g.heap.pop().map(|p| p.req)
    }

    /// Snapshot the pending URLs (for checkpoint).
    pub async fn pending_urls(&self) -> Vec<Request> {
        let g = self.heap.lock();
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

    /// Number of pending requests.
    pub async fn len(&self) -> usize {
        self.heap.lock().heap.len()
    }

    /// 队列是否为空。
    pub async fn is_empty(&self) -> bool {
        self.heap.lock().heap.is_empty()
    }

    /// Replace inner state (for checkpoint restore).
    pub async fn restore(&self, pending: Vec<Request>, seen: HashSet<String>) {
        // 清 seen（DashSet）
        self.seen_exact.clear();
        self.seen_fp.clear();
        // 清 heap + seq（Mutex）
        {
            let mut g = self.heap.lock();
            g.heap.clear();
            g.seq = 0;
        }
        // Rebuild seen set
        for url in &seen {
            match self.strategy {
                DedupStrategy::Exact => {
                    self.seen_exact.insert(url.clone());
                }
                DedupStrategy::Fingerprint => {
                    // seen_urls() 在 Fingerprint 模式下返回 u64 哈希的十进制字符串，
                    // 直接 parse 回 u64 即可，不能再 fingerprint（会产生不同 u64）。
                    if let Ok(h) = url.parse::<u64>() {
                        self.seen_fp.insert(h);
                    }
                }
            }
        }
        // Re-queue pending (force insert even if in seen set)
        let mut g = self.heap.lock();
        for req in pending {
            match self.strategy {
                DedupStrategy::Exact => {
                    self.seen_exact.insert(req.url.clone());
                }
                DedupStrategy::Fingerprint => {
                    self.seen_fp.insert(fingerprint(&req.url));
                }
            }
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seq += 1;
        }
    }
}

// Add Clone bound for PrioritizedRequest (needed by pending_urls)
impl Clone for PrioritizedRequest {
    fn clone(&self) -> Self {
        Self {
            req: self.req.clone(),
            seq: self.seq,
        }
    }
}

fn fingerprint(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}

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
        let sched = Scheduler::with_strategy(DedupStrategy::Fingerprint);
        // push 两个 URL：进入 heap 与 seen_fp
        sched.push(Request::get("https://example.com/a")).await;
        sched.push(Request::get("https://example.com/b")).await;
        // pop 模拟已爬取：heap 清空，但 seen_fp 保留正确指纹
        sched.pop().await;
        sched.pop().await;

        // 快照 seen（checkpoint 持久化的就是 seen 状态）
        let seen = sched.seen_urls().await;
        assert_eq!(seen.len(), 2, "快照应含 2 个哈希字符串");

        // 此时 heap 已空，pending 为空——模拟纯 seen 往返
        let pending = sched.pending_urls().await;
        assert!(pending.is_empty(), "pop 后 pending 应为空");
        sched.restore(pending, seen).await;

        // 再 push 同样的 URL：应被 seen 判定为已爬，不入 heap
        let before = sched.len().await;
        sched.push(Request::get("https://example.com/a")).await;
        let after = sched.len().await;
        assert_eq!(
            before, after,
            "Fingerprint 模式下 restore 后 seen 应仍能去重，实际 before={before}, after={after}"
        );
    }
}
