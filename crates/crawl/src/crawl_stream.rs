//! 流式爬取事件与事件流包装。

use crate::CrawlStats;
use crate::Item;
use wisp_fetcher::FetchMode;

/// 爬取过程中的事件流。
#[derive(Debug, Clone)]
pub enum CrawlEvent {
    /// 爬取启动。
    CrawlStarted {
        /// Spider 名称。
        spider: String,
        /// 起始 URL 数量。
        start_urls: usize,
    },
    /// 单个 Spider 爬取完成。
    CrawlFinished {
        /// 统计快照。
        stats: CrawlStats,
    },
    /// 响应接收。
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
    /// 爬取到的 item。
    Item(Item),
    /// 页面爬取完成。
    PageScraped {
        /// 页面 URL。
        url: String,
        /// 当前统计。
        stats: CrawlStats,
    },
    /// 重试发生。`attempt` 从 1 开始，`max` 为重试上限。
    Retry {
        /// 请求 URL。
        url: String,
        /// 当前重试次数。
        attempt: u32,
        /// 最大重试次数。
        max: u32,
        /// 错误信息。
        error: String,
    },
    /// 最终失败。
    Error {
        /// 请求 URL。
        url: String,
        /// 错误信息。
        error: String,
        /// 失败时的重试次数。
        attempt: u32,
    },
    /// 检测到封锁。
    BlockedDetected {
        /// 请求 URL。
        url: String,
        /// 状态码。
        status: u16,
    },
    /// Auto 模式升级。
    AutoUpgraded {
        /// 请求 URL。
        url: String,
        /// 原模式。
        from: FetchMode,
        /// 新模式。
        to: FetchMode,
    },
    /// Checkpoint 已保存。
    CheckpointSaved {
        /// 待处理请求数。
        pending: usize,
    },
    /// 单 Spider 流爬取完成。
    Done(CrawlStats),
    /// 多 Spider 流爬取完成。
    DoneMany(Vec<CrawlStats>),
}

/// 流式爬取事件流
pub struct CrawlStream {
    pub(crate) inner: std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent> + Send>>,
}

impl CrawlStream {
    /// 过滤出 item 流。
    pub fn items(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = Item> + Send>> {
        use futures::StreamExt;
        Box::pin(self.inner.filter_map(|e| async move { match e {
            CrawlEvent::Item(item) => Some(item),
            _ => None,
        }}))
    }
    /// 获取完整事件流。
    pub fn events(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent> + Send>> {
        self.inner
    }
}
