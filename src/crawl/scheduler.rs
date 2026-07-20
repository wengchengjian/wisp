//! URL scheduler with priority queue and deduplication.

use std::collections::{BinaryHeap, HashSet};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use super::SpiderRequest;

struct PrioritizedRequest {
    req: SpiderRequest,
    seq: u64,  // tie-breaker for FIFO within same priority
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
            .then_with(|| other.seq.cmp(&self.seq))  // lower seq = earlier = higher priority
    }
}

/// URL scheduler with deduplication.
pub struct Scheduler {
    heap: BinaryHeap<PrioritizedRequest>,
    seen: HashSet<u64>,
    seq: u64,
}

impl Scheduler {
    pub fn new() -> Self { Self { heap: BinaryHeap::new(), seen: HashSet::new(), seq: 0 } }

    /// Push a request (deduplicates by URL fingerprint).
    pub fn push(&mut self, req: SpiderRequest) {
        let fp = fingerprint(&req.url);
        if self.seen.insert(fp) {
            self.heap.push(PrioritizedRequest { req, seq: self.seq });
            self.seq += 1;
        }
    }

    /// Pop the highest-priority request.
    pub fn pop(&mut self) -> Option<SpiderRequest> {
        self.heap.pop().map(|p| p.req)
    }

    pub fn len(&self) -> usize { self.heap.len() }
    pub fn is_empty(&self) -> bool { self.heap.is_empty() }
}

fn fingerprint(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}
