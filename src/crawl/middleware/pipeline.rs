//! 内建 Item Pipeline 实现。

use std::future::Future;
use std::pin::Pin;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use super::{ItemPipeline, CrawlContext};

/// JSONL 文件写入管道。
///
/// 使用 `open` 打开文件句柄，`close` 关闭，中间复用同一句柄。
pub struct JsonlWriterPipeline {
    path: String,
    file: Mutex<Option<std::fs::File>>,
}

impl JsonlWriterPipeline {
    pub fn new(path: &str) -> Self {
        Self { path: path.to_string(), file: Mutex::new(None) }
    }
}

#[async_trait]
impl ItemPipeline for JsonlWriterPipeline {
    async fn open(&self, _ctx: &CrawlContext) {
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            *self.file.lock().await = Some(file);
        }
    }

    async fn process_item(&self, item: Value, _ctx: &CrawlContext) -> Option<Value> {
        use std::io::Write;
        let mut guard = self.file.lock().await;
        if let Some(ref mut file) = *guard {
            if let Ok(line) = serde_json::to_string(&item) {
                let _ = writeln!(file, "{}", line);
            }
        } else if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            if let Ok(line) = serde_json::to_string(&item) {
                let _ = writeln!(file, "{}", line);
            }
        }
        Some(item)
    }

    async fn close(&self, _ctx: &CrawlContext) {
        use std::io::Write;
        let mut guard = self.file.lock().await;
        if let Some(ref mut file) = *guard {
            let _ = file.flush();
        }
        *guard = None;
    }
}

/// 字段过滤管道：仅保留指定字段。
pub struct FilterFieldsPipeline {
    fields: Vec<String>,
}

impl FilterFieldsPipeline {
    pub fn new(fields: Vec<&str>) -> Self {
        Self { fields: fields.into_iter().map(|s| s.to_string()).collect() }
    }
}

#[async_trait]
impl ItemPipeline for FilterFieldsPipeline {
    async fn process_item(&self, item: Value, _ctx: &CrawlContext) -> Option<Value> {
        match item {
            Value::Object(map) => {
                let filtered: serde_json::Map<String, Value> = map
                    .into_iter()
                    .filter(|(k, _)| self.fields.contains(k))
                    .collect();
                Some(Value::Object(filtered))
            }
            other => Some(other),
        }
    }
}

/// 通用批量处理 Pipeline：内部缓冲，满 batch_size 条自动调用 flush_fn 批量提交。
///
/// 适用于数据库批量 INSERT、文件批量写入、API 批量上报等场景。
///
/// # 示例
///
/// ```rust,no_run
/// use wisp::crawl::middleware::BatchItemPipeline;
///
/// let pipeline = BatchItemPipeline::new(100, |items| async move {
///     // 批量写入逻辑
///     println!("flushing {} items", items.len());
/// });
/// ```
pub struct BatchItemPipeline {
    buffer: Mutex<Vec<Value>>,
    batch_size: usize,
    flush_fn: Box<dyn Fn(Vec<Value>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>,
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
            let batch = std::mem::take(&mut *buf);
            (self.flush_fn)(batch).await;
        }
    }
}
