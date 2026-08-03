//! Crawl stats: public value snapshot, runtime counters, checkpoint state.
//!
//! `CrawlStats` is the one public stats model. `SpiderStats` is the engine's
//! internal counter adapter. `CrawlState` embeds `CrawlStats` for persistence.

use dashmap::DashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Duration 序列化为 u64 毫秒，避免 checkpoint 与流事件快照携带非 serde 类型。
mod duration_ms {
    use super::*;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

/// Crawling statistics.
///
/// 由 [`SpiderStats`] 派生的只读快照，只保留引擎实际统计的字段。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrawlStats {
    /// 已爬取 item 数。
    pub items_scraped: usize,
    /// 已爬取页面数。
    pub pages_crawled: usize,
    /// 错误数。
    pub errors: usize,
    /// 总耗时。
    #[serde(with = "duration_ms")]
    pub duration: Duration,
    /// 被拦截请求数。
    pub blocked_requests: usize,
    /// 重试次数。
    pub retry_count: usize,
    /// 状态码分布。
    pub status_code_counts: HashMap<u16, usize>,
    /// 站外请求数。
    pub offsite_requests_count: usize,
    /// 缓存命中数。
    pub cache_hits: usize,
    /// 各 callback 已爬页数（`"default"` 表示无 callback 的入口请求）。
    #[serde(default)]
    pub callback_pages: HashMap<String, usize>,
}

impl CrawlStats {
    /// 生成摘要字符串。
    pub fn summary(&self) -> String {
        format!(
            "爬取完成: {} 页 / {} items / {} 错误 / 耗时 {:?}",
            self.pages_crawled, self.items_scraped, self.errors, self.duration
        )
    }
}

/// Serializable crawl state for checkpoint persistence.
///
/// Stored as bincode blob in SQLite `crawl_checkpoints` table.
/// Stats live in one embedded CrawlStats value; scheduling fields stay here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlState {
    /// Spider 名称。
    pub spider_name: String,
    /// 待处理 URL 列表。
    pub pending_urls: Vec<crate::Request>,
    /// 已访问 URL 集合。
    pub seen_urls: HashSet<String>,
    /// 统计快照。
    pub stats: CrawlStats,
    /// 保存时仍在处理中的请求（重启后重新入队，保证至少一次语义）。
    #[serde(default)]
    pub in_flight_urls: Vec<crate::Request>,
    /// 保存时间。
    pub saved_at: chrono::DateTime<chrono::Utc>,
}

impl CrawlState {
    /// 创建新的爬取状态。
    pub fn new(spider_name: String) -> Self {
        Self {
            spider_name,
            pending_urls: Vec::new(),
            seen_urls: HashSet::new(),
            stats: CrawlStats::default(),
            in_flight_urls: Vec::new(),
            saved_at: chrono::Utc::now(),
        }
    }

    /// 还原为 CrawlStats。
    pub fn to_stats(&self) -> CrawlStats {
        self.stats.clone()
    }
}

/// 单个 Spider 的运行时统计。引擎为每个 Spider 持有一个实例。
pub(crate) struct SpiderStats {
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
    /// 已耗时。
    pub fn elapsed(&self) -> Duration {
        let offset = Duration::from_millis(self.elapsed_offset_ms.load(Ordering::SeqCst));
        self.start.elapsed() + offset
    }

    /// 从 checkpoint 状态恢复计数。
    pub fn restore_from(&self, state: &CrawlState) {
        self.restore(&state.to_stats());
    }

    /// 生成当前计数的完整 CrawlStats 快照。
    pub fn snapshot(&self) -> CrawlStats {
        CrawlStats {
            items_scraped: self.items.load(Ordering::SeqCst),
            pages_crawled: self.pages.load(Ordering::SeqCst),
            errors: self.errors.load(Ordering::SeqCst),
            duration: self.elapsed(),
            blocked_requests: self.blocked.load(Ordering::SeqCst),
            retry_count: self.retries.load(Ordering::SeqCst),
            status_code_counts: self.status_codes_snapshot(),
            offsite_requests_count: self.offsite.load(Ordering::SeqCst),
            cache_hits: self.cache_hits.load(Ordering::SeqCst),
            callback_pages: self.callback_pages_snapshot(),
        }
    }

    /// 从 CrawlStats 快照恢复计数。
    pub fn restore(&self, stats: &CrawlStats) {
        self.pages.store(stats.pages_crawled, Ordering::SeqCst);
        self.items.store(stats.items_scraped, Ordering::SeqCst);
        self.errors.store(stats.errors, Ordering::SeqCst);
        self.blocked.store(stats.blocked_requests, Ordering::SeqCst);
        self.retries.store(stats.retry_count, Ordering::SeqCst);
        self.offsite
            .store(stats.offsite_requests_count, Ordering::SeqCst);
        self.cache_hits.store(stats.cache_hits, Ordering::SeqCst);
        self.status_codes.clear();
        for (status, count) in &stats.status_code_counts {
            self.status_codes.insert(*status, AtomicUsize::new(*count));
        }
        self.elapsed_offset_ms
            .store(stats.duration.as_millis() as u64, Ordering::SeqCst);
        self.callback_pages.clear();
        for (callback, count) in &stats.callback_pages {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crawl_stats_serde_roundtrips_duration_and_callback_pages() {
        let stats = CrawlStats {
            pages_crawled: 7,
            items_scraped: 3,
            duration: Duration::from_millis(1234),
            status_code_counts: HashMap::from([(200, 7)]),
            callback_pages: HashMap::from([("detail".to_string(), 2)]),
            ..Default::default()
        };
        let bytes = bincode::serialize(&stats).unwrap();
        let decoded: CrawlStats = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.pages_crawled, 7);
        assert_eq!(decoded.duration, Duration::from_millis(1234));
        assert_eq!(decoded.callback_pages.get("detail"), Some(&2));
        assert_eq!(decoded.status_code_counts.get(&200), Some(&7));
    }

    #[test]
    fn checkpoint_stats_roundtrip_through_bincode() {
        let stats = SpiderStats::new();
        stats.pages.store(5, Ordering::SeqCst);
        stats.items.store(9, Ordering::SeqCst);
        stats.errors.store(1, Ordering::SeqCst);
        stats.blocked.store(2, Ordering::SeqCst);
        stats.retries.store(3, Ordering::SeqCst);
        stats.offsite.store(4, Ordering::SeqCst);
        stats.cache_hits.store(6, Ordering::SeqCst);
        stats.record_status(200);
        stats.record_status(200);
        stats.record_callback_page("detail");
        stats.record_callback_page("detail");

        std::thread::sleep(Duration::from_millis(2));
        let snapshot = stats.snapshot();
        let state = CrawlState {
            stats: snapshot.clone(),
            ..CrawlState::new("s".into())
        };
        let bytes = bincode::serialize(&state).unwrap();
        let decoded: CrawlState = bincode::deserialize(&bytes).unwrap();

        let restored = SpiderStats::new();
        restored.restore(&decoded.stats);
        assert_eq!(restored.pages.load(Ordering::SeqCst), 5);
        assert_eq!(restored.items.load(Ordering::SeqCst), 9);
        assert_eq!(restored.errors.load(Ordering::SeqCst), 1);
        assert_eq!(restored.blocked.load(Ordering::SeqCst), 2);
        assert_eq!(restored.retries.load(Ordering::SeqCst), 3);
        assert_eq!(restored.offsite.load(Ordering::SeqCst), 4);
        assert_eq!(restored.cache_hits.load(Ordering::SeqCst), 6);
        assert_eq!(restored.status_codes_snapshot(), HashMap::from([(200, 2)]));
        assert_eq!(
            restored.callback_pages_snapshot(),
            HashMap::from([("detail".to_string(), 2)])
        );
        assert!(decoded.stats.duration.as_millis() > 0);
        assert_eq!(
            decoded.stats.duration.as_millis(),
            snapshot.duration.as_millis()
        );
    }

    #[tokio::test]
    async fn status_codes_concurrent_increment_is_correct() {
        let stats = Arc::new(SpiderStats::new());
        let handles: Vec<_> = (0..50)
            .map(|_| {
                let s = stats.clone();
                tokio::spawn(async move {
                    for _ in 0..100 {
                        s.record_status(200);
                        s.record_status(404);
                    }
                })
            })
            .collect();
        for h in handles {
            h.await.unwrap();
        }

        let snap = stats.status_codes_snapshot();
        assert_eq!(snap.get(&200).copied(), Some(5000), "200 计数应为 50*100");
        assert_eq!(snap.get(&404).copied(), Some(5000), "404 计数应为 50*100");
        assert_eq!(snap.len(), 2, "仅 2 个状态码");
    }

    #[tokio::test]
    async fn status_codes_snapshot_reflects_recorded_status() {
        let stats = Arc::new(SpiderStats::new());
        assert!(
            stats.status_codes_snapshot().is_empty(),
            "fresh stats snapshot 应为空"
        );

        stats.record_status(200);
        stats.record_status(200);
        stats.record_status(500);

        let snap = stats.status_codes_snapshot();
        assert_eq!(snap.len(), 2, "应含 2 个状态码");
        assert_eq!(snap.get(&200).copied(), Some(2), "200 计数应为 2");
        assert_eq!(snap.get(&500).copied(), Some(1), "500 计数应为 1");
    }
}
