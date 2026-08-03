//! 字段过滤管道。

use async_trait::async_trait;
use serde_json::Value;

use crate::Item;
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
    async fn process_item(&self, item: Item<Value>, _ctx: &CrawlContext) -> Option<Item<Value>> {
        Some(item.map_value(|value| {
            match value {
                Value::Object(map) => Value::Object(
                    map.into_iter()
                        .filter(|(k, _)| self.fields.contains(k))
                        .collect(),
                ),
                other => other,
            }
        }))
    }
}
