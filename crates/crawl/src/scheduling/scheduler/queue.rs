//! 优先级队列内部类型。

use crate::CrawlRequest;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub(super) struct PrioritizedRequest {
    pub(super) req: CrawlRequest,
    pub(super) seq: u64,
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
pub(super) struct HeapInner {
    pub(super) heap: BinaryHeap<PrioritizedRequest>,
    pub(super) seq: u64,
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
