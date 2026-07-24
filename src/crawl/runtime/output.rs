//! Content output formats: Markdown and WARC.
//!
//! Provides converters and streaming writers for exporting crawled pages
//! in LLM-friendly Markdown or archival WARC/1.1 format.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use crate::error::{WispError, Result};
use crate::utils::{status_text, url_to_filename};

/// Output format enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Jsonl,
    Markdown,
    Warc,
}

/// Convert HTML to Markdown using htmd.
pub fn html_to_markdown(html: &str) -> Result<String> {
    let converter = htmd::HtmlToMarkdown::new();
    converter.convert(html)
        .map_err(|e| WispError::ParseError(format!("html2markdown: {e}")))
}

/// Build a WARC/1.1 response record.
pub fn to_warc_record(url: &str, status: u16, headers: &HashMap<String, String>, body: &[u8]) -> String {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let http_headers: String = headers.iter()
        .map(|(k, v)| format!("{}: {}\r\n", k, v))
        .collect();
    let http_block = format!(
        "HTTP/1.1 {} {}\r\n{}\r\n",
        status,
        status_text(status),
        http_headers,
    );
    let http_block_bytes = http_block.as_bytes();
    let content_length = http_block_bytes.len() + body.len();

    format!(
        "WARC/1.1\r\n\
         WARC-Type: response\r\n\
         WARC-Target-URI: {}\r\n\
         WARC-Date: {}\r\n\
         Content-Type: application/http; msgtype=response\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}{}",
        url,
        now,
        content_length,
        http_block,
        String::from_utf8_lossy(body),
    )
}



/// Streaming WARC writer (appends records to a file).
pub struct WarcWriter {
    file: std::fs::File,
}

impl WarcWriter {
    pub fn new(path: &Path) -> Result<Self> {
        Ok(Self { file: std::fs::File::create(path)? })
    }

    pub fn write_response(&mut self, url: &str, status: u16, headers: &HashMap<String, String>, body: &[u8]) -> Result<()> {
        use std::io::Write;
        let record = to_warc_record(url, status, headers, body);
        writeln!(self.file, "{}\r\n", record)?;
        Ok(())
    }

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
    pub fn new(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self { dir: dir.to_path_buf(), counter: 0 })
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
        assert!(record.starts_with("WARC/1.1\r\n"));
        assert!(record.contains("WARC-Target-URI: https://example.com"));
        assert!(record.contains("HTTP/1.1 200 OK"));
        assert!(record.contains("<html>hi</html>"));
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
        let path = writer.write_page("https://example.com/test", "<h1>Hi</h1><p>Content</p>").unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Hi"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
