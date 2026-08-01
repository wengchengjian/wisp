//! JSONL 文件写入管道。

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::middleware::{CrawlContext, ItemPipeline};

pub struct JsonlWriterPipeline {
    path: String,
    file: Mutex<Option<std::fs::File>>,
}

impl JsonlWriterPipeline {
    /// 创建 JSONL 写入管道。
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            file: Mutex::new(None),
        }
    }
}

#[async_trait]
impl ItemPipeline for JsonlWriterPipeline {
    async fn open(&self, _ctx: &CrawlContext) {
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
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
        } else if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
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
