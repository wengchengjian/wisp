//! 内建 Item Pipeline 实现。

use async_trait::async_trait;
use serde_json::Value;

use super::ItemPipeline;

/// JSONL 文件写入管道。
pub struct JsonlWriterPipeline {
    path: String,
}

impl JsonlWriterPipeline {
    pub fn new(path: &str) -> Self {
        Self { path: path.to_string() }
    }
}

#[async_trait]
impl ItemPipeline for JsonlWriterPipeline {
    async fn process_item(&self, item: Value) -> Option<Value> {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            if let Ok(line) = serde_json::to_string(&item) {
                let _ = writeln!(file, "{}", line);
            }
        }
        Some(item)
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
    async fn process_item(&self, item: Value) -> Option<Value> {
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
