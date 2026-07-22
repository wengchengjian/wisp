//! Per-spider 统计计数器。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// 单个 Spider 的运行时统计。引擎为每个 Spider 持有一个实例。
pub struct SpiderStats {
    pub pages: AtomicUsize,
    pub items: AtomicUsize,
    pub errors: AtomicUsize,
    pub blocked: AtomicUsize,
    pub retries: AtomicUsize,
    pub offsite: AtomicUsize,
    pub cache_hits: AtomicUsize,
    pub in_flight: AtomicUsize,
    pub status_codes: Mutex<HashMap<u16, usize>>,
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
            in_flight: AtomicUsize::new(0),
            status_codes: Mutex::new(HashMap::new()),
            start: Instant::now(),
        }
    }

    pub fn pages(&self) -> usize { self.pages.load(Ordering::SeqCst) }
    pub fn items(&self) -> usize { self.items.load(Ordering::SeqCst) }
    pub fn errors(&self) -> usize { self.errors.load(Ordering::SeqCst) }
    pub fn in_flight(&self) -> usize { self.in_flight.load(Ordering::SeqCst) }
    pub fn elapsed(&self) -> Duration { self.start.elapsed() }
}

impl Default for SpiderStats {
    fn default() -> Self { Self::new() }
}
