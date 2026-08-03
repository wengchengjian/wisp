//! MCP adaptive_scrape 工具。

use std::sync::Arc;

use super::types::{AdaptiveScrapeArgs, AdaptiveScrapeResult, ToolContext};
use wisp_core::error::Result;
use wisp_core::{FetchMode, Request};
use wisp_parser::Node;
use wisp_storage::{Store, open_store};

/// 自适应抓取：CSS 失败时用快照存储重定位。
pub async fn adaptive_scrape(
    args: AdaptiveScrapeArgs,
    ctx: &ToolContext<'_>,
) -> Result<AdaptiveScrapeResult> {
    wisp_core::utils::validate_url(&args.url)?;
    let resp = ctx
        .fetch_client
        .fetch(&Request::get(&args.url), FetchMode::Http)
        .await?;
    let html = resp.text()?;
    let doc = Node::from_html(&html);
    let effective_store: Arc<dyn Store> = match args.db_path.as_deref() {
        Some(path) if !path.is_empty() => open_store(path)?,
        _ => Arc::clone(ctx.store),
    };
    let tracker = wisp_crawl::AdaptiveTracker::new(effective_store);
    let found = tracker
        .css_adaptive(
            &doc,
            &args.selector,
            &args.key,
            &args.url,
            true,
            wisp_parser::DEFAULT_TOLERANCE,
        )
        .await?;
    match found {
        Some(node) => Ok(AdaptiveScrapeResult {
            url: args.url,
            found: true,
            text: Some(node.text()),
            html: Some(node.html()),
        }),
        None => Ok(AdaptiveScrapeResult {
            url: args.url,
            found: false,
            text: None,
            html: None,
        }),
    }
}
