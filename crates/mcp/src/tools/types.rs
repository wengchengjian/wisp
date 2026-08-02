//! MCP tool typed arguments, results and shared tool context.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use wisp_crawl::Engine;
use wisp_fetcher::FetchClient;
use wisp_storage::Store;

/// Shared resources available to every MCP tool.
pub struct ToolContext<'a> {
    /// Persistence store for checkpoint/adaptive snapshot tools.
    pub store: &'a Arc<dyn Store>,
    /// Shared crawl Engine.
    pub engine: &'a Engine,
    /// Shared FetchClient for HTTP/browser/stealth transports.
    pub fetch_client: &'a Arc<FetchClient>,
}

/// `fetch_page` arguments.
#[derive(Debug, Deserialize)]
pub struct FetchPageArgs {
    /// Target URL.
    pub url: String,
    /// Optional TLS fingerprint profile name.
    #[serde(default)]
    pub emulation: Option<String>,
}

/// `fetch_page` result.
#[derive(Debug, Serialize)]
pub struct FetchPageResult {
    /// Final URL.
    pub url: String,
    /// HTTP status code.
    pub status: u16,
    /// Decoded HTML.
    pub html: String,
    /// Raw body byte count.
    pub bytes: usize,
}

/// `extract_css` arguments.
#[derive(Debug, Deserialize)]
pub struct ExtractCssArgs {
    /// HTML source.
    pub html: String,
    /// CSS selector.
    pub selector: String,
    /// Optional attribute name to extract.
    #[serde(default)]
    pub attr: Option<String>,
}

/// `extract_css` result.
#[derive(Debug, Serialize)]
pub struct ExtractCssResult {
    /// Extracted text values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<String>,
    /// Extracted attribute values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attrs: Vec<String>,
}

/// `crawl_site` arguments.
#[derive(Debug, Deserialize)]
pub struct CrawlSiteArgs {
    /// Seed URLs.
    pub start_urls: Vec<String>,
    /// CSS selector used by the built-in spider.
    pub css_selector: String,
    /// Per-call page cap.
    #[serde(default)]
    pub max_pages: Option<u64>,
    /// Optional link-following regex.
    #[serde(default)]
    pub follow_pattern: Option<String>,
    /// Maximum follow depth.
    #[serde(default)]
    pub max_depth: Option<u64>,
    /// Optional domain allowlist.
    #[serde(default)]
    pub allowed_domains: Option<Vec<String>>,
}

/// `crawl_site` result.
#[derive(Debug, Serialize)]
pub struct CrawlSiteResult {
    /// Number of items produced.
    pub items_count: usize,
    /// JSONL representation of items.
    pub jsonl: String,
}

/// `adaptive_scrape` arguments.
#[derive(Debug, Deserialize)]
pub struct AdaptiveScrapeArgs {
    /// Target URL.
    pub url: String,
    /// CSS selector.
    pub selector: String,
    /// Stable element key.
    pub key: String,
    /// Optional snapshot store path.
    #[serde(default)]
    pub db_path: Option<String>,
}

/// `adaptive_scrape` result.
#[derive(Debug, Serialize)]
pub struct AdaptiveScrapeResult {
    /// Target URL.
    pub url: String,
    /// Whether the element was found.
    pub found: bool,
    /// Extracted text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Extracted HTML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}

/// `stealth_fetch` arguments.
#[cfg(feature = "stealth")]
#[derive(Debug, Deserialize)]
pub struct StealthFetchArgs {
    /// Target URL.
    pub url: String,
}

/// `stealth_fetch` result.
#[cfg(feature = "stealth")]
#[derive(Debug, Serialize)]
pub struct StealthFetchResult {
    /// Final URL.
    pub url: String,
    /// Page title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Decoded HTML.
    pub html: String,
    /// Raw body byte count.
    pub bytes: usize,
}
