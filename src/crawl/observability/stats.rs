//! Per-spider 统计计数器。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use dashmap::DashMap;

/// 单个 Spider 的运行时统计。引擎为每个 Spider 持有一个实例。
pub struct SpiderStats {
    pub pages: AtomicUsize,
    pub items: AtomicUsize,
    pub errors: AtomicUsize,
    pub blocked: AtomicUsize,
    pub retries: AtomicUsize,
    pub offsite: AtomicUsize,
    pub cache_hits: AtomicUsize,
    /// 在飞请求数。使用 Arc 以便 InFlightGuard 克隆。
    pub in_flight: Arc<AtomicUsize>,
    pub status_codes: DashMap<u16, AtomicUsize>,
    pub start: Instant,
}

impl SpiderStats {
    pub fn new() -> Self {
        Self {
            pages: AtomicUsize::new(0),
            items: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            blocked: AtomicUsize::new(0),
            retries: AtomicUsize::new(0),
            offsite: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            in_flight: Arc::new(AtomicUsize::new(0)),
            status_codes: DashMap::new(),
            start: Instant::now(),
        }
    }

    pub fn pages(&self) -> usize { self.pages.load(Ordering::SeqCst) }
    pub fn items(&self) -> usize { self.items.load(Ordering::SeqCst) }
    pub fn errors(&self) -> usize { self.errors.load(Ordering::SeqCst) }
    pub fn in_flight(&self) -> usize { self.in_flight.load(Ordering::SeqCst) }
    pub fn elapsed(&self) -> Duration { self.start.elapsed() }

    /// 无锁快照状态码计数为 HashMap<u16, usize>。
    pub fn status_codes_snapshot(&self) -> HashMap<u16, usize> {
        self.status_codes
            .iter()
            .map(|r| (*r.key(), r.value().load(Ordering::SeqCst)))
            .collect()
    }
}

impl Default for SpiderStats {
    fn default() -> Self { Self::new() }
}
