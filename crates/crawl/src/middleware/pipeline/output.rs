//! 按 OutputFormat 写入文件的输出管道。

use async_trait::async_trait;
use serde_json::Value;

use crate::Item;
use crate::middleware::{CrawlContext, ItemPipeline};
use crate::runtime::output::{ItemOutput, OutputFormat};
use wisp_core::error::Result;

/// 按 `OutputFormat` 将 item 写入文件的通用管道。
///
/// - `Json`：流式 JSON 数组（`open` 写 `[`，`close` 写 `]`），序列化整个 Item
/// - `Jsonl`：每行一个 Item
/// - `Markdown`：每页一个 `.md` 文件（`path` 为目录，使用 `item.source_url()` 与 `html`）
/// - `Warc`：WARC/1.1 记录追加写入（使用 `item.source_url()` 与 `html`）
pub struct OutputWriterPipeline {
    output: ItemOutput,
}

impl OutputWriterPipeline {
    /// 创建输出管道，打开时覆盖目标文件。`path` 对 Markdown 是输出目录，其余格式是输出文件路径。
    pub fn new(format: OutputFormat, path: &str) -> Self {
        Self {
            output: ItemOutput::new(format, path),
        }
    }

    /// 创建追加模式的输出管道（仅对 JSONL 有意义）。
    pub fn new_append(format: OutputFormat, path: &str) -> Self {
        Self {
            output: ItemOutput::with_append(format, path, true),
        }
    }
}

#[async_trait]
impl ItemPipeline for OutputWriterPipeline {
    async fn open(&self, _ctx: &CrawlContext) -> Result<()> {
        self.output.open().await
    }

    async fn process_item(
        &self,
        item: Item<Value>,
        _ctx: &CrawlContext,
    ) -> Result<Option<Item<Value>>> {
        self.output.write(&item).await?;
        Ok(Some(item))
    }

    async fn close(&self, _ctx: &CrawlContext) -> Result<()> {
        self.output.close().await
    }
}
