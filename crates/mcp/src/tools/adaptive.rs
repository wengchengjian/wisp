//! MCP adaptive_scrape 工具。

use std::sync::Arc;

use super::fetch_html::fetch_html;
use super::types::{AdaptiveScrapeArgs, AdaptiveScrapeResult, ToolContext};
use wisp_core::FetchMode;
use wisp_core::error::Result;
use wisp_parser::Node;
use wisp_storage::{Store, open_store};

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
