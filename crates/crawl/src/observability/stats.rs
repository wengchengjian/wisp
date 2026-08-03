//! Per-spider 统计计数器。

use crate::CrawlState;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// 单个 Spider 的运行时统计。引擎为每个 Spider 持有一个实例。
pub struct SpiderStats {
    /// 已爬取页面数。
    pub pages: AtomicUsize,
    /// 已爬取 item 数。
    pub items: AtomicUsize,
    /// 错误数。
    pub errors: AtomicUsize,
    /// 被拦截数。
    pub blocked: AtomicUsize,
    /// 重试数。
    pub retries: AtomicUsize,
    /// 站外请求数。
    pub offsite: AtomicUsize,
    /// 缓存命中数。
    pub cache_hits: AtomicUsize,
    /// 在飞请求数。使用 Arc 以便 InFlightGuard 克隆。
    pub in_flight: Arc<AtomicUsize>,
    /// 状态码计数。
    pub status_codes: DashMap<u16, AtomicUsize>,
    /// callback 维度已爬页数（`"default"` 表示无 callback 的入口请求）。
    pub callback_pages: DashMap<String, usize>,
    /// 开始时间。
    pub start: Instant,
    /// checkpoint 恢复的累计耗时偏移（供 Timeout 等 until 条件续接）。
    elapsed_offset_ms: AtomicU64,
}

impl SpiderStats {
    /// 创建新的统计实例。
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
            callback_pages: DashMap::new(),
            start: Instant::now(),
            elapsed_offset_ms: AtomicU64::new(0),
        }
    }

    /// 记录一个 callback 页面已爬取。
    pub fn record_callback_page(&self, callback: &str) {
        let mut entry = self.callback_pages.entry(callback.to_string()).or_insert(0);
        *entry += 1;
    }

    /// 无锁快照 callback 页面计数为 HashMap<String, usize>。
    pub fn callback_pages_snapshot(&self) -> HashMap<String, usize> {
        self.callback_pages
            .iter()
            .map(|r| (r.key().clone(), *r.value()))
            .collect()
    }

    /// 已爬取页面数。
    pub fn pages(&self) -> usize {
        self.pages.load(Ordering::SeqCst)
    }
    /// 已爬取 item 数。
    pub fn items(&self) -> usize {
        self.items.load(Ordering::SeqCst)
    }
    /// 错误数。
    pub fn errors(&self) -> usize {
        self.errors.load(Ordering::SeqCst)
    }
    /// 在飞请求数。
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }
    /// 已耗时。
    pub fn elapsed(&self) -> Duration {
        let offset = Duration::from_millis(self.elapsed_offset_ms.load(Ordering::SeqCst));
        self.start.elapsed() + offset
    }

    /// 从 checkpoint 状态恢复计数（页面/item/错误与 callback 维度计数）。
    pub fn restore_from(&self, state: &CrawlState) {
        self.pages.store(state.pages_crawled, Ordering::SeqCst);
        self.items.store(state.items_scraped, Ordering::SeqCst);
        self.errors.store(state.errors, Ordering::SeqCst);
        self.blocked.store(state.blocked, Ordering::SeqCst);
        self.retries.store(state.retries, Ordering::SeqCst);
        self.offsite.store(state.offsite, Ordering::SeqCst);
        self.cache_hits.store(state.cache_hits, Ordering::SeqCst);
        self.status_codes.clear();
        for (status, count) in &state.status_codes {
            self.status_codes.insert(*status, AtomicUsize::new(*count));
        }
        self.elapsed_offset_ms
            .store(state.duration_ms as u64, Ordering::SeqCst);
        self.callback_pages.clear();
        for (callback, count) in &state.callback_pages {
            self.callback_pages.insert(callback.clone(), *count);
        }
    }

    /// 记录一个 HTTP 状态码（无锁累加）。
    pub fn record_status(&self, status: u16) {
        self.status_codes
            .entry(status)
            .and_modify(|c| {
                c.fetch_add(1, Ordering::Relaxed);
            })
            .or_insert(AtomicUsize::new(1));
    }

    /// 无锁快照状态码计数为 HashMap<u16, usize>。
    pub fn status_codes_snapshot(&self) -> HashMap<u16, usize> {
        self.status_codes
            .iter()
            .map(|r| (*r.key(), r.value().load(Ordering::SeqCst)))
            .collect()
    }
}

impl Default for SpiderStats {
    fn default() -> Self {
        Self::new()
    }
}
