//! Content output formats: JSON, JSONL, Markdown and WARC.
//!
//! `ItemOutput` is the single streaming writer behind file-backed Item output;
//! `Items` is the in-memory view that retains Item provenance.

use crate::Item;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
use wisp_core::error::{Result, WispError};
use wisp_core::utils::{status_text, url_to_filename};

/// Output format enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// JSON 格式。
    Json,
    /// JSONL 格式（每行一个 JSON）。
    Jsonl,
    /// Markdown 格式。
    Markdown,
    /// WARC 归档格式。
    Warc,
}

fn serialize_error(e: serde_json::Error) -> WispError {
    WispError::Parse(wisp_core::error::ParseError::Serialize(e.to_string()))
}

/// Serialize a batch of Items as pretty JSON.
pub fn items_to_json(items: &[Item<Value>]) -> Result<String> {
    serde_json::to_string_pretty(items).map_err(serialize_error)
}

/// Serialize a batch of Items as JSONL (one Item per line).
pub fn items_to_jsonl(items: &[Item<Value>]) -> Result<String> {
    let mut out = String::new();
    for item in items {
        let line = serde_json::to_string(item).map_err(serialize_error)?;
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// Merge HTML/text payloads into one Markdown document.
pub fn items_to_markdown(items: &[Item<Value>]) -> Result<String> {
    let mut out = String::new();
    for item in items {
        if let Some(html) = item.value().get("html").and_then(Value::as_str) {
            out.push_str(&html_to_markdown(html)?);
            out.push_str("\n\n---\n\n");
        } else if let Some(text) = item.value().get("text").and_then(Value::as_str) {
            out.push_str(text);
            out.push('\n');
        }
    }
    Ok(out)
}

/// Crawl result collection that keeps Item provenance.
pub struct Items {
    items: Vec<Item<Value>>,
}

impl Items {
    /// Create an Items collection from Items.
    pub fn new(items: Vec<Item<Value>>) -> Self {
        Self { items }
    }

    /// Create an Items collection from Items (compatibility alias).
    pub fn from_items(items: Vec<Item<Value>>) -> Self {
        Self::new(items)
    }

    /// Create an Items collection from raw payloads with empty provenance.
    pub fn from_values(values: Vec<Value>) -> Self {
        Self {
            items: values
                .into_iter()
                .map(|v| Item::new(v, "", "", None))
                .collect(),
        }
    }

    /// Item 数量。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 返回 Item 迭代器。
    pub fn iter(&self) -> impl Iterator<Item = &Item<Value>> {
        self.items.iter()
    }

    /// 导出为 JSON 字符串（pretty）
    pub fn to_json(&self) -> Result<String> {
        items_to_json(&self.items)
    }

    /// 导出为 JSONL（每行一个 JSON 对象）
    pub fn to_jsonl(&self) -> Result<String> {
        items_to_jsonl(&self.items)
    }

    /// 写入 JSON 文件
    pub fn to_json_file(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }

    /// 写入 JSONL 文件
    pub fn to_jsonl_file(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_jsonl()?)?;
        Ok(())
    }

    /// 将所有 item 中的 "html" 字段转为 markdown，返回合并结果。
    pub fn to_markdown(&self) -> Result<String> {
        items_to_markdown(&self.items)
    }
}

enum Inner {
    Json { file: std::fs::File, first: bool },
    Jsonl(std::fs::File),
    Markdown(MarkdownWriter),
    Warc(WarcWriter),
}

/// Streaming Item writer for one output format.
///
/// `new` truncates the destination on open; `with_append(..., true)` appends
/// (meaningful for JSONL). Errors propagate instead of being swallowed.
pub struct ItemOutput {
    format: OutputFormat,
    path: PathBuf,
    append: bool,
    inner: Mutex<Option<Inner>>,
}

impl ItemOutput {
    /// Create a writer that overwrites the destination on open.
    pub fn new(format: OutputFormat, path: &str) -> Self {
        Self::with_append(format, path, false)
    }

    /// Create a writer with an explicit append flag.
    pub fn with_append(format: OutputFormat, path: &str, append: bool) -> Self {
        Self {
            format,
            path: PathBuf::from(path),
            append,
            inner: Mutex::new(None),
        }
    }

    /// Open the destination. No-op when already open.
    pub async fn open(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let inner = match self.format {
            OutputFormat::Json => {
                use std::io::Write;
                let mut file = std::fs::File::create(&self.path)?;
                file.write_all(b"[")?;
                Inner::Json { file, first: true }
            }
            OutputFormat::Jsonl => {
                let file = if self.append {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&self.path)?
                } else {
                    std::fs::File::create(&self.path)?
                };
                Inner::Jsonl(file)
            }
            OutputFormat::Markdown => Inner::Markdown(MarkdownWriter::new(&self.path)?),
            OutputFormat::Warc => Inner::Warc(WarcWriter::new(&self.path)?),
        };
        *guard = Some(inner);
        Ok(())
    }

    /// Write one Item. Returns an error when the destination is not open.
    pub async fn write(&self, item: &Item<Value>) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let Some(inner) = guard.as_mut() else {
            return Err(WispError::Io(std::io::Error::other(
                "ItemOutput is not open",
            )));
        };
        match inner {
            Inner::Json { file, first } => {
                use std::io::Write;
                if !*first {
                    file.write_all(b",")?;
                }
                let json = serde_json::to_string(item).map_err(serialize_error)?;
                file.write_all(json.as_bytes())?;
                *first = false;
            }
            Inner::Jsonl(file) => {
                use std::io::Write;
                let json = serde_json::to_string(item).map_err(serialize_error)?;
                writeln!(file, "{json}")?;
            }
            Inner::Markdown(writer) => {
                if let Some(html) = item.value().get("html").and_then(Value::as_str) {
                    writer.write_page(item.source_url(), html)?;
                }
            }
            Inner::Warc(writer) => {
                if let Some(html) = item.value().get("html").and_then(Value::as_str) {
                    writer.write_response(
                        item.source_url(),
                        200,
                        &HashMap::new(),
                        html.as_bytes(),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Flush and close the destination. No-op when already closed.
    pub async fn close(&self) -> Result<()> {
        use std::io::Write;
        let mut guard = self.inner.lock().await;
        let Some(mut inner) = guard.take() else {
            return Ok(());
        };
        match &mut inner {
            Inner::Json { file, .. } => {
                writeln!(file, "]")?;
                file.flush()?;
            }
            Inner::Jsonl(file) => file.flush()?,
            Inner::Markdown(_) => {}
            Inner::Warc(writer) => writer.flush()?,
        }
        Ok(())
    }
}

/// Convert HTML to Markdown using htmd.
pub fn html_to_markdown(html: &str) -> Result<String> {
    let converter = htmd::HtmlToMarkdown::new();
    converter.convert(html).map_err(|e| {
        WispError::Parse(wisp_core::error::ParseError::Html(format!(
            "html2markdown: {e}"
        )))
    })
}

/// Build a WARC/1.1 response record.
pub fn to_warc_record(
    url: &str,
    status: u16,
    headers: &HashMap<String, String>,
    body: &[u8],
) -> Vec<u8> {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let clean = |s: &str| s.replace(['\r', '\n'], " ");
    let sanitized_url = clean(url);
    let http_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{}: {}\r\n", clean(k), clean(v)))
        .collect();
    let http_block = format!(
        "HTTP/1.1 {} {}\r\n{}\r\n",
        status,
        status_text(status),
        http_headers,
    );
    let http_block_bytes = http_block.as_bytes();
    let content_length = http_block_bytes.len() + body.len();

    let mut out = Vec::with_capacity(256 + http_block_bytes.len() + body.len());
    out.extend_from_slice(
        format!(
            "WARC/1.1\r\nWARC-Type: response\r\nWARC-Target-URI: {}\r\nWARC-Date: {}\r\nContent-Type: application/http; msgtype=response\r\nContent-Length: {}\r\n\r\n",
            sanitized_url, now, content_length
        )
        .as_bytes(),
    );
    out.extend_from_slice(http_block_bytes);
    out.extend_from_slice(body);
    out
}

/// Streaming WARC writer (appends records to a file).
pub struct WarcWriter {
    file: std::fs::File,
}

impl WarcWriter {
    /// 创建 WARC 写入器。
    pub fn new(path: &Path) -> Result<Self> {
        Ok(Self {
            file: std::fs::File::create(path)?,
        })
    }

    /// 写入响应记录。
    pub fn write_response(
        &mut self,
        url: &str,
        status: u16,
        headers: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<()> {
        use std::io::Write;
        let record = to_warc_record(url, status, headers, body);
        self.file.write_all(&record)?;
        self.file.write_all(b"\r\n\r\n")?;
        Ok(())
    }

    /// 刷新缓冲区。
    pub fn flush(&mut self) -> Result<()> {
        use std::io::Write;
        self.file.flush()?;
        Ok(())
    }
}

/// Streaming Markdown writer (one .md file per page).
pub struct MarkdownWriter {
    dir: PathBuf,
    counter: usize,
}

impl MarkdownWriter {
    /// 创建 Markdown 写入器。
    pub fn new(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            counter: 0,
        })
    }

    /// Convert HTML to Markdown and write to a file. Returns the output path.
    pub fn write_page(&mut self, url: &str, html: &str) -> Result<PathBuf> {
        let md = html_to_markdown(html)?;
        // Generate filename from URL or counter
        let safe_name = url_to_filename(url, self.counter);
        self.counter += 1;
        let path = self.dir.join(safe_name);
        std::fs::write(&path, &md)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_html_to_markdown_basic() {
        let html = "<h1>Title</h1><p>Hello <strong>world</strong></p>";
        let md = html_to_markdown(html).unwrap();
        assert!(md.contains("Title"), "should contain title: {}", md);
        assert!(md.contains("world"), "should contain world: {}", md);
    }

    #[test]
    fn test_warc_record_format() {
        let headers = HashMap::from([("Content-Type".to_string(), "text/html".to_string())]);
        let record = to_warc_record("https://example.com", 200, &headers, b"<html>hi</html>");
        let record = String::from_utf8(record).expect("ASCII record");
        assert!(record.starts_with("WARC/1.1\r\n"));
        assert!(record.contains("WARC-Target-URI: https://example.com"));
        assert!(record.contains("HTTP/1.1 200 OK"));
        assert!(record.contains("<html>hi</html>"));
    }

    #[test]
    fn warc_record_preserves_binary_body() {
        let body = [0u8, 255, 13, 10];
        let record = to_warc_record("https://example.com", 200, &HashMap::new(), &body);
        assert_eq!(&record[record.len() - body.len()..], &body);
    }

    #[test]
    fn warc_record_sanitizes_crlf_in_headers() {
        let headers = HashMap::from([("X-Bad".to_string(), "a\r\nInjected: yes".to_string())]);
        let record = to_warc_record("https://example.com", 200, &headers, b"ok");
        let text = String::from_utf8_lossy(&record);
        assert!(!text.contains("\r\nInjected"), "header 注入必须被清洗");
    }

    #[test]
    fn test_url_to_filename() {
        let name = url_to_filename("https://example.com/books/123", 0);
        assert!(name.ends_with(".md"));
        assert!(name.contains("example.com"));
    }

    #[test]
    fn test_markdown_writer() {
        let dir = std::env::temp_dir().join("wisp_test_md_output");
        let _ = std::fs::remove_dir_all(&dir);
        let mut writer = MarkdownWriter::new(&dir).unwrap();
        let path = writer
            .write_page("https://example.com/test", "<h1>Hi</h1><p>Content</p>")
            .unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Hi"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn items_to_json_preserves_provenance() {
        let item = Item::new(
            json!({"a": 1}),
            "https://example.com/x",
            "spider",
            Some("cb"),
        );
        let items = Items::new(vec![item]);
        let s = items.to_json().unwrap();
        assert!(s.contains("https://example.com/x"));
        assert!(s.contains("\"spider\""));
    }

    #[test]
    fn items_to_jsonl_writes_one_line_per_item() {
        let a = Item::new(json!({"a": 1}), "https://example.com/a", "s", None);
        let b = Item::new(json!({"b": 2}), "https://example.com/b", "s", None);
        let items = Items::new(vec![a, b]);
        let s = items.to_jsonl().unwrap();
        assert_eq!(s.trim_end().lines().count(), 2);
    }

    #[test]
    fn items_from_values_wraps_empty_provenance() {
        let items = Items::from_values(vec![json!({"x": 1})]);
        assert_eq!(items.len(), 1);
        assert_eq!(items.iter().next().unwrap().source_url(), "");
    }

    #[tokio::test]
    async fn item_output_writes_jsonl_with_provenance() {
        let suffix = wisp_core::utils::random::rand_suffix();
        let path = std::env::temp_dir().join(format!("wisp_item_output_{suffix}.jsonl"));
        let output = ItemOutput::new(OutputFormat::Jsonl, path.to_str().unwrap());
        output.open().await.unwrap();
        output
            .write(&Item::new(
                json!({"x": 1}),
                "https://example.com/x",
                "s",
                None,
            ))
            .await
            .unwrap();
        output.close().await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("https://example.com/x"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn item_output_open_propagates_io_error() {
        let suffix = wisp_core::utils::random::rand_suffix();
        let path = std::env::temp_dir().join(format!("wisp_missing_{suffix}/out.jsonl"));
        let output = ItemOutput::new(OutputFormat::Jsonl, path.to_str().unwrap());
        assert!(output.open().await.is_err());
    }
}
