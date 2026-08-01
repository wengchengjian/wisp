//! 字段过滤管道。

use async_trait::async_trait;
use serde_json::Value;

use crate::middleware::{CrawlContext, ItemPipeline};

/// 字段过滤管道：仅保留指定字段。
pub struct FilterFieldsPipeline {
    fields: Vec<String>,
}

impl FilterFieldsPipeline {
    /// 创建字段过滤管道。
    pub fn new(fields: Vec<&str>) -> Self {
        Self {
            fields: fields.into_iter().map(|s| s.to_string()).collect(),
        }
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
