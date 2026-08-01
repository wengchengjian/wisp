//! CrawlStats 只读统计快照。

use std::collections::HashMap;
use std::time::Duration;

/// Crawling statistics.
///
/// 由 [`crate::observability::stats::SpiderStats`] 派生的只读快照，
/// 只保留引擎实际统计的字段，不再维护与运行期计数重复的死数据。
#[derive(Debug, Clone, Default)]
pub struct CrawlStats {
    /// 已爬取 item 数。
    pub items_scraped: usize,
    /// 已爬取页面数。
    pub pages_crawled: usize,
    /// 错误数。
    pub errors: usize,
    /// 总耗时。
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
