//! 按 OutputFormat 写入文件的输出管道。

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::middleware::{CrawlContext, ItemPipeline};
use crate::runtime::output::{MarkdownWriter, OutputFormat, WarcWriter};

/// 按 `OutputFormat` 将 item 写入文件的通用管道。
///
/// - `Json`：流式 JSON 数组（`open` 写 `[`，`close` 写 `]`）
/// - `Jsonl`：每行一个 JSON
/// - `Markdown`：每页一个 `.md` 文件（`path` 为目录，item 需含 `url`/`html`）
/// - `Warc`：WARC/1.1 记录追加写入（item 需含 `url`/`html`）
pub struct OutputWriterPipeline {
    format: OutputFormat,
    path: String,
    file: Mutex<Option<(std::fs::File, bool)>>,
    markdown: Mutex<Option<MarkdownWriter>>,
    warc: Mutex<Option<WarcWriter>>,
}

impl OutputWriterPipeline {
    /// 创建输出管道。`path` 对 Markdown 是输出目录，其余格式是输出文件路径。
    pub fn new(format: OutputFormat, path: &str) -> Self {
        Self {
            format,
            path: path.to_string(),
            file: Mutex::new(None),
            markdown: Mutex::new(None),
            warc: Mutex::new(None),
        }
    }

    async fn write_json_item(&self, item: &Value) {
        use std::io::Write;
        let mut guard = self.file.lock().await;
        if let Some((ref mut file, ref mut first)) = *guard {
            if !*first {
                let _ = write!(file, ",");
            }
            if let Ok(json) = serde_json::to_string(item) {
                let _ = write!(file, "{}", json);
            }
            *first = false;
        }
    }

    async fn write_jsonl_item(&self, item: &Value) {
        use std::io::Write;
        let mut guard = self.file.lock().await;
        if let Some((ref mut file, _)) = *guard {
            if let Ok(json) = serde_json::to_string(item) {
                let _ = writeln!(file, "{}", json);
            }
        }
    }

    async fn write_markdown_item(&self, item: &Value) {
        let mut guard = self.markdown.lock().await;
        if let Some(ref mut writer) = *guard {
            let url = item.get("url").and_then(Value::as_str).unwrap_or("page");
            if let Some(html) = item.get("html").and_then(Value::as_str) {
                let _ = writer.write_page(url, html);
            }
        }
    }

    async fn write_warc_item(&self, item: &Value) {
        let mut guard = self.warc.lock().await;
        if let Some(ref mut writer) = *guard {
            let url = item.get("url").and_then(Value::as_str).unwrap_or("page");
            if let Some(html) = item.get("html").and_then(Value::as_str) {
                let _ = writer.write_response(
                    url,
                    200,
                    &std::collections::HashMap::new(),
                    html.as_bytes(),
                );
            }
        }
    }
}

#[async_trait]
impl ItemPipeline for OutputWriterPipeline {
    async fn open(&self, _ctx: &CrawlContext) {
        use std::io::Write;
        match self.format {
            OutputFormat::Json => {
                if let Ok(mut file) = std::fs::File::create(&self.path) {
                    let _ = write!(file, "[");
                    *self.file.lock().await = Some((file, true));
                }
            }
            OutputFormat::Jsonl => {
                if let Ok(file) = std::fs::File::create(&self.path) {
                    *self.file.lock().await = Some((file, true));
                }
            }
            OutputFormat::Markdown => {
                if let Ok(writer) = MarkdownWriter::new(std::path::Path::new(&self.path)) {
                    *self.markdown.lock().await = Some(writer);
                }
            }
            OutputFormat::Warc => {
                if let Ok(writer) = WarcWriter::new(std::path::Path::new(&self.path)) {
                    *self.warc.lock().await = Some(writer);
                }
            }
        }
    }

    async fn process_item(&self, item: Value, _ctx: &CrawlContext) -> Option<Value> {
        match self.format {
            OutputFormat::Json => self.write_json_item(&item).await,
            OutputFormat::Jsonl => self.write_jsonl_item(&item).await,
            OutputFormat::Markdown => self.write_markdown_item(&item).await,
            OutputFormat::Warc => self.write_warc_item(&item).await,
        }
        Some(item)
    }

    async fn close(&self, _ctx: &CrawlContext) {
        use std::io::Write;
        if let Some((ref mut file, _)) = *self.file.lock().await {
            if self.format == OutputFormat::Json {
                let _ = write!(file, "]\n");
            }
            let _ = file.flush();
        }
        if let Some(ref mut writer) = *self.warc.lock().await {
            let _ = writer.flush();
        }
        *self.file.lock().await = None;
        *self.markdown.lock().await = None;
        *self.warc.lock().await = None;
    }
}
