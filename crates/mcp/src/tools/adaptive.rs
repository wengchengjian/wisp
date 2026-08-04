//! MCP adaptive_scrape 工具。

use std::future::Future;
use std::pin::Pin;

use super::fetch_html::fetch_html;
use super::types::ToolContext;
use crate::protocol::{Tool, TypedRun};
use serde::{Deserialize, Serialize};
use serde_json::json;
use wisp_core::FetchMode;
use wisp_core::error::Result;
use wisp_parser::Node;

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

/// 自适应抓取：CSS 失败时用快照存储重定位。
pub async fn adaptive_scrape(
    args: AdaptiveScrapeArgs,
    ctx: &ToolContext<'_>,
) -> Result<AdaptiveScrapeResult> {
    let page = fetch_html(
        ctx,
        &args.url,
        FetchMode::Http,
        &wisp_fetcher::FetchOptions::default(),
    )
    .await?;
    let doc = Node::from_html(&page.html);
    let effective_store = ctx.get_or_open_store(args.db_path.as_deref())?;
    let tracker = wisp_crawl::AdaptiveTracker::new(effective_store);
    let found = tracker
        .css_adaptive(
            &doc,
            &args.selector,
            &args.key,
            &page.url,
            true,
            wisp_parser::DEFAULT_TOLERANCE,
        )
        .await?;
    match found {
        Some(node) => Ok(AdaptiveScrapeResult {
            url: page.url,
            found: true,
            text: Some(node.text()),
            html: Some(node.html()),
        }),
        None => Ok(AdaptiveScrapeResult {
            url: page.url,
            found: false,
            text: None,
            html: None,
        }),
    }
}

fn adaptive_scrape_run<'a>(
    args: AdaptiveScrapeArgs,
    ctx: &'a ToolContext<'a>,
) -> Pin<Box<dyn Future<Output = Result<AdaptiveScrapeResult>> + Send + 'a>> {
    Box::pin(adaptive_scrape(args, ctx))
}

pub(crate) fn spec() -> Tool {
    Tool::from_handler(
        "adaptive_scrape",
        "自适应抓取：CSS 失败时用 SQLite 快照重定位元素（长期监控）。",
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "selector": { "type": "string" },
                "key": { "type": "string", "description": "元素稳定标识" },
                "db_path": { "type": "string", "default": "./wisp.db" }
            },
            "required": ["url", "selector", "key"]
        }),
        adaptive_scrape_run as TypedRun<AdaptiveScrapeArgs, AdaptiveScrapeResult>,
    )
}
