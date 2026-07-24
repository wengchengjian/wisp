//! Crawl state for checkpoint persistence.
//!
//! Stored as bincode blob in SQLite `crawl_checkpoints` table.
//! `CrawlStats.duration: Duration` 不实现 serde，所以 CrawlState 拆开
//! stats 为标量字段 + duration_ms，避免修改 CrawlStats 的 derive。

use std::collections::HashSet;
use serde::{Serialize, Deserialize};
use crate::crawl::{Request, CrawlStats};

/// Serializable crawl state for checkpoint persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlState {
    pub spider_name: String,
    pub pending_urls: Vec<Request>,
    pub seen_urls: HashSet<String>,
    pub items_scraped: usize,
    pub pages_crawled: usize,
    pub errors: usize,
    /// 爬取累计时长（毫秒）。`std::time::Duration` 不实现 serde，
    /// 用 u128 毫秒往返（足够精度，无溢出风险）。
    pub duration_ms: u128,
    pub saved_at: chrono::DateTime<chrono::Utc>,
}

impl CrawlState {
    pub fn new(spider_name: String) -> Self {
        Self {
            spider_name,
            pending_urls: Vec::new(),
            seen_urls: HashSet::new(),
            items_scraped: 0,
            pages_crawled: 0,
            errors: 0,
            duration_ms: 0,
            saved_at: chrono::Utc::now(),
        }
    }

    /// 从 CrawlStats 构造（snapshot 用）。
    pub fn from_stats(spider_name: String, stats: &CrawlStats, pending: Vec<Request>) -> Self {
        Self {
            spider_name,
            pending_urls: pending,
            seen_urls: HashSet::new(), // stage 1: not tracked separately
            items_scraped: stats.items_scraped,
            pages_crawled: stats.pages_crawled,
            errors: stats.errors,
            duration_ms: stats.duration.as_millis(),
            saved_at: chrono::Utc::now(),
        }
    }

    /// 还原为 CrawlStats。
    pub fn to_stats(&self) -> CrawlStats {
        CrawlStats {
            items_scraped: self.items_scraped,
            pages_crawled: self.pages_crawled,
            errors: self.errors,
            duration: std::time::Duration::from_millis(self.duration_ms as u64),
            ..Default::default()
        }
    }
}
