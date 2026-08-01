//! 流式爬取事件与事件流包装。

use crate::crawl_stats::CrawlStats;
use serde_json::Value;

/// 爬取过程中的事件流
#[derive(Debug, Clone)]
pub enum CrawlEvent {
    /// 爬取到的 item。
    Item(Value),
    /// 页面爬取完成。
    PageScraped {
        /// 页面 URL。
        url: String,
        /// 当前统计。
        stats: CrawlStats,
    },
    /// 错误发生。
    Error {
        /// 请求 URL。
        url: String,
        /// 错误信息。
        error: String,
    },
    /// ND-001-ERR：重试事件，让 stream 消费者感知重试发生。
    /// `attempt` 为当前重试次数（从 1 开始），`max` 为上限，`error` 为失败原因。
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
    /// 爬取完成。
    Done(CrawlStats),
    /// 多 Spider 流式爬取完成。
    DoneMany(Vec<CrawlStats>),
}

/// 流式爬取事件流
pub struct CrawlStream {
    pub(crate) inner: std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>>,
}

impl CrawlStream {
    /// 过滤出 item 流。
    pub fn items(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = Value>>> {
        use futures::StreamExt;
        Box::pin(self.inner.filter_map(|e| async move {
            match e {
                CrawlEvent::Item(v) => Some(v),
                _ => None,
            }
        }))
    }
    /// 获取完整事件流。
    pub fn events(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>> {
        self.inner
    }
}
