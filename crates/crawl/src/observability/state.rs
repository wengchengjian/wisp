//! Crawl state for checkpoint persistence.
//!
//! Stored as bincode blob in SQLite `crawl_checkpoints` table.
//! `CrawlStats.duration: Duration` 不实现 serde，所以 CrawlState 拆开
//! stats 为标量字段 + duration_ms，避免修改 CrawlStats 的 derive。

use crate::{CrawlStats, Request};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;

/// Serializable crawl state for checkpoint persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlState {
    /// Spider 名称。
    pub spider_name: String,
    /// 待处理 URL 列表。
    pub pending_urls: Vec<Request>,
    /// 已访问 URL 集合。
    pub seen_urls: HashSet<String>,
    /// 已爬取 item 数。
    pub items_scraped: usize,
    /// 已爬取页面数。
    pub pages_crawled: usize,
    /// 错误数。
    pub errors: usize,
    /// 状态码分布。
    #[serde(default)]
    pub status_codes: HashMap<u16, usize>,
    /// 被拦截数。
    #[serde(default)]
    pub blocked: usize,
    /// 重试数。
    #[serde(default)]
    pub retries: usize,
    /// 站外请求数。
    #[serde(default)]
    pub offsite: usize,
    /// 缓存命中数。
    #[serde(default)]
    pub cache_hits: usize,
    /// 各 callback 已爬页数（`"default"` 表示无 callback 的入口请求）。
    #[serde(default)]
    pub callback_pages: HashMap<String, usize>,
    /// 保存时仍在处理中的请求（重启后重新入队，保证至少一次语义）。
    #[serde(default)]
    pub in_flight_urls: Vec<Request>,
    /// 爬取累计时长（毫秒）。`std::time::Duration` 不实现 serde，
    /// 用 u128 毫秒往返（足够精度，无溢出风险）。
    pub duration_ms: u128,
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
            items_scraped: 0,
            pages_crawled: 0,
            errors: 0,
            status_codes: HashMap::new(),
            blocked: 0,
            retries: 0,
            offsite: 0,
            cache_hits: 0,
            callback_pages: HashMap::new(),
            in_flight_urls: Vec::new(),
            duration_ms: 0,
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
            blocked_requests: self.blocked,
            retry_count: self.retries,
            status_code_counts: self.status_codes.clone(),
            offsite_requests_count: self.offsite,
            cache_hits: self.cache_hits,
        }
    }
}
