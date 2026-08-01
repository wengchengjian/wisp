//! Content output formats: Markdown and WARC.
//!
//! Provides converters and streaming writers for exporting crawled pages
//! in LLM-friendly Markdown or archival WARC/1.1 format.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
        let headers = HashMap::from([(
            "X-Bad".to_string(),
            "a\r\nInjected: yes".to_string(),
        )]);
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
}
