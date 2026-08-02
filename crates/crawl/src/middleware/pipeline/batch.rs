//! 通用批量处理管道。

use async_trait::async_trait;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::Mutex;

use crate::middleware::{CrawlContext, ItemPipeline};

type FlushFn = Box<dyn Fn(Vec<Value>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// 通用批量处理 Pipeline：内部缓冲，满 batch_size 条自动调用 flush_fn 批量提交。
///
/// 适用于数据库批量 INSERT、文件批量写入、API 批量上报等场景。
///
/// # 示例
///
/// ```rust,no_run
/// use wisp_crawl::middleware::BatchItemPipeline;
///
/// let pipeline = BatchItemPipeline::new(100, async |items| {
///     // 批量写入逻辑
///     println!("flushing {} items", items.len());
/// });
/// ```
pub struct BatchItemPipeline {
    buffer: Mutex<Vec<Value>>,
    batch_size: usize,
    flush_fn: FlushFn,
}

impl BatchItemPipeline {
    /// 创建批量 Pipeline。
    ///
    /// - `batch_size`：缓冲区大小，满时自动 flush
    /// - `flush_fn`：批量提交逻辑（接收一批 items）
    pub fn new<F, Fut>(batch_size: usize, flush_fn: F) -> Self
    where
        F: Fn(Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self {
            buffer: Mutex::new(Vec::with_capacity(batch_size)),
            batch_size,
            flush_fn: Box::new(move |items| Box::pin(flush_fn(items))),
        }
    }
}

#[async_trait]
impl ItemPipeline for BatchItemPipeline {
    async fn process_item(&self, item: Value, _ctx: &CrawlContext) -> Option<Value> {
        let mut buf = self.buffer.lock().await;
        buf.push(item.clone());
        if buf.len() >= self.batch_size {
            let batch = std::mem::take(&mut *buf);
            (self.flush_fn)(batch).await;
        }
        Some(item)
    }

    async fn close(&self, _ctx: &CrawlContext) {
        let mut buf = self.buffer.lock().await;
        if !buf.is_empty() {
            let remaining = buf.len();
            tracing::info!(
                "BatchItemPipeline::close: flushing {} remaining items",
                remaining
            );
            let batch = std::mem::take(&mut *buf);
            (self.flush_fn)(batch).await;
        } else {
            tracing::debug!("BatchItemPipeline::close: buffer empty, nothing to flush");
        }
    }
}
