//! Scheduler 实现。

mod restore;
mod snapshot;

use super::dedup::{DedupStrategy, fingerprint};
use super::queue::{HeapInner, PrioritizedRequest};
use crate::Request;
use dashmap::DashSet;
use std::collections::{BinaryHeap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use tokio::sync::Mutex;

/// Scheduler：seen 集合（DashSet，无锁）与 heap（独立 Mutex）分离。
///
/// push 时先查/插 seen（DashSet，无锁），命中才锁 heap 入队；
/// pop 时只锁 heap。两者不再串行于同一锁。
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
    /// 待处理请求数（无锁读取，避免主循环每次 `len()` 都锁 heap）。
    pending: Arc<AtomicUsize>,
    /// 已发出过告警的标记，避免重复日志刷屏（每次超过只告警一次）。
    warned: Arc<std::sync::atomic::AtomicBool>,
}

impl Scheduler {
    /// 创建默认调度器（精确去重）。
    pub fn new() -> Self {
        Self::with_strategy(DedupStrategy::Exact)
    }

    /// 使用指定去重策略创建 Scheduler。
    pub fn with_strategy(strategy: DedupStrategy) -> Self {
        Self::with_strategy_and_max_seen(strategy, usize::MAX)
    }

    /// ND-008-ARCH：创建 Scheduler 并设置 seen 集合容量告警阈值。
    ///
    /// 当 seen 集合大小超过 `max_seen` 时，记录一次 warn 日志。
    /// 默认 `usize::MAX` 表示无告警。建议大规模爬取设置为 1_000_000。
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
            pending: Arc::new(AtomicUsize::new(0)),
            warned: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn insert_seen(&self, req: &Request) -> bool {
        match self.strategy {
            DedupStrategy::Exact => self.seen_exact.insert(req.url.clone()),
            DedupStrategy::Fingerprint => self.seen_fp.insert(fingerprint(&req.url)),
        }
    }

    fn warn_if_seen_large(&self) {
        if self.max_seen == usize::MAX {
            return;
        }
        let current_size = match self.strategy {
            DedupStrategy::Exact => self.seen_exact.len(),
            DedupStrategy::Fingerprint => self.seen_fp.len(),
        };
        if current_size > self.max_seen
            && !self.warned.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            tracing::warn!(
                "Scheduler seen 集合已超过告警阈值 {} (当前 {})",
                self.max_seen,
                current_size
            );
        }
    }

    async fn enqueue(&self, req: Request) {
        let mut g = self.heap.lock().await;
        let seq = g.seq;
        g.heap.push(PrioritizedRequest { req, seq });
        g.seq += 1;
        self.pending.fetch_add(1, AtomicOrdering::Relaxed);
    }

    /// Push a request (deduplicates by URL).
    pub async fn push(&self, req: Request) {
        if !self.insert_seen(&req) {
            return;
        }
        self.warn_if_seen_large();
        self.enqueue(req).await;
    }

    /// Pop the highest-priority request.
    pub async fn pop(&self) -> Option<Request> {
        let mut g = self.heap.lock().await;
        let req = g.heap.pop().map(|p| p.req);
        if req.is_some() {
            self.pending.fetch_sub(1, AtomicOrdering::Relaxed);
        }
        req
    }

    /// Number of pending requests.
    pub async fn len(&self) -> usize {
        self.pending.load(AtomicOrdering::Relaxed)
    }

    /// 队列是否为空。
    pub async fn is_empty(&self) -> bool {
        self.pending.load(AtomicOrdering::Relaxed) == 0
    }
}
