//! 引擎内部事件定义。

use crate::CrawlStats;
use wisp_fetcher::FetchMode;

/// 引擎内部事件。
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// 爬取启动
    CrawlStarted {
        /// Spider 名称。
        spider: String,
        /// 起始 URL 数量。
        start_urls: usize,
    },
    /// 爬取完成
    CrawlFinished {
        /// 统计信息。
        stats: CrawlStats,
    },
    /// 请求被调度
    RequestScheduled {
        /// 请求 URL。
        url: String,
        /// 深度。
        depth: u32,
    },
    /// 响应接收
    ResponseReceived {
        /// 请求 URL。
        url: String,
        /// 状态码。
        status: u16,
        /// 耗时（毫秒）。
        elapsed_ms: u64,
        /// 是否来自缓存。
        from_cache: bool,
    },
    /// Item 产出
    ItemScraped {
        /// 来源 URL。
        url: String,
    },
    /// 错误发生
    ErrorOccurred {
        /// 请求 URL。
        url: String,
        /// 错误信息。
        error: String,
        /// 重试次数。
        attempt: u32,
    },
    /// 检测到封锁
    BlockedDetected {
        /// 请求 URL。
        url: String,
        /// 状态码。
        status: u16,
    },
    /// Auto 模式升级
    AutoUpgraded {
        /// 请求 URL。
        url: String,
        /// 原模式。
        from: FetchMode,
        /// 新模式。
        to: FetchMode,
    },
    /// 并发数变更
    ConcurrencyChanged {
        /// 原并发数。
        old: usize,
        /// 新并发数。
        new: usize,
    },
    /// Checkpoint 保存
    CheckpointSaved {
        /// 待处理请求数。
        pending: usize,
    },
}
