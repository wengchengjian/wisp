//! MCP stealth_fetch 工具：复用共享 FetchClient + StealthStrategy。

#[cfg(feature = "stealth")]
use serde_json::{json, Value};
#[cfg(feature = "stealth")]
use std::sync::Arc;
#[cfg(feature = "stealth")]
use wisp_core::error::{McpError, Result, WispError};
#[cfg(feature = "stealth")]
use wisp_core::Request;
#[cfg(feature = "stealth")]
use wisp_fetcher::{cookie::CfCookieJar, FetchClient, StealthStrategy};

#[cfg(feature = "stealth")]
pub async fn stealth_fetch(args: Value, fetch_client: &Arc<FetchClient>) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'url'".into())))?;
    wisp_core::utils::validate_url(url)?;
    let config = fetch_client.config();
    let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
    let strategy = StealthStrategy::from_config(config, cf_jar);
    let req = Request::get(url);
    let resp = fetch_client
        .fetch_browser(&req, &strategy)
        .await
        .map_err(|e| WispError::Mcp(McpError::General(format!("stealth fetch: {e}"))))?;
    let html = String::from_utf8_lossy(&resp.body).to_string();
    Ok(json!({
        "url": resp.url,
        "title": resp.title,
        "html": html,
        "bytes": resp.body.len()
    }))
}
