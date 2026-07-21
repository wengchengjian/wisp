//! MCP 工具实现。

use serde_json::Value;
use std::sync::Arc;
use crate::error::Result;
use crate::storage::Store;

pub async fn fetch_page(args: Value) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("fetch_page not implemented yet".into()))
}

pub async fn extract_css(args: Value) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("extract_css not implemented yet".into()))
}

pub async fn extract_xpath(args: Value) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("extract_xpath not implemented yet".into()))
}

pub async fn crawl_site(args: Value, _store: &Arc<Store>) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("crawl_site not implemented yet".into()))
}

pub async fn adaptive_scrape(args: Value, _store: &Arc<Store>) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("adaptive_scrape not implemented yet".into()))
}

pub async fn stealth_fetch(args: Value) -> Result<Value> {
    let _ = args;
    Err(crate::error::WispError::McpError("stealth_fetch not implemented yet".into()))
}
