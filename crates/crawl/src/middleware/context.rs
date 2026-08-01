//! 引擎上下文只读视图。

use wisp_fetcher::FetchMode;

/// 引擎上下文只读视图（暴露给中间件）。
///
/// 中间件可读取引擎级配置和统计信息，用于决策（如根据已爬取页数调整策略）。
#[derive(Debug, Clone)]
pub struct CrawlContext {
    /// Spider 名称
    pub spider_name: String,
    /// 当前抓取模式
    pub fetch_mode: FetchMode,
    /// 最大并发数
    pub max_concurrent: usize,
    /// 最大爬取页数
    pub max_pages: usize,
    /// 是否遵守 robots.txt
    pub obey_robots: bool,
    /// 已爬取页数（只读快照）
    pub pages_crawled: usize,
    /// 错误数（只读快照）
    pub errors: usize,
}
