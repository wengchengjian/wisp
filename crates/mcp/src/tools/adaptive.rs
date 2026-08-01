//! MCP adaptive_scrape 工具。

use serde_json::{json, Value};
use std::sync::Arc;
use wisp_core::error::{McpError, Result, WispError};
use wisp_http::Client;
use wisp_parser::Node;
use wisp_storage::Store;

/// 自适应抓取：CSS 失败时用 SQLite 快照重定位。
async fn adaptive_scrape_impl(
    url: &str,
    selector: &str,
    key: &str,
    store: &Arc<dyn Store>,
) -> Result<Value> {
    wisp_core::utils::validate_url(url)?;
    let client = Client::builder().build()?;
    let resp = client.get(url, &[]).await?;
    let html = resp.text()?;
    let doc = Node::from_html(&html);
    use wisp_crawl::AdaptiveTracker;
    let tracker = AdaptiveTracker::new(Arc::clone(store));
    let found = tracker
        .css_adaptive(
            &doc,
            selector,
            key,
            url,
            true,
            wisp_parser::DEFAULT_TOLERANCE,
        )
        .await?;
    match found {
        Some(node) => Ok(json!({
            "url": url,
            "found": true,
            "text": node.text(),
            "html": node.html()
        })),
        None => Ok(json!({ "url": url, "found": false })),
    }
}

pub async fn adaptive_scrape(args: Value, store: &Arc<dyn Store>) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'url'".into())))?;
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'selector'".into())))?;
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'key'".into())))?;
    adaptive_scrape_impl(url, selector, key, store).await
}
