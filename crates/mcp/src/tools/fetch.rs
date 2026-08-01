//! MCP fetch_page 工具。

use serde_json::{json, Value};
use wisp_core::error::{McpError, Result, WispError};
use wisp_http::Client;
use wreq_util::Profile;

/// 抓取单个网页，返回 HTML 文本。
pub async fn fetch_page(args: Value) -> Result<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::Mcp(McpError::General("missing 'url' argument".into())))?;

    wisp_core::utils::validate_url(url)?;

    let mut builder = Client::builder();
    if let Some(emu) = args.get("emulation").and_then(|v| v.as_str()) {
        let profile = match emu {
            "firefox" => Profile::Firefox128,
            "safari" => Profile::Safari18,
            _ => Profile::Chrome136,
        };
        builder = builder.emulation(profile);
    }

    let client = builder.build()?;
    let resp = client.get(url, &[]).await?;
    let html = resp.text()?;

    Ok(json!({
        "url": url,
        "status": resp.status,
        "html": html,
        "bytes": resp.body.len()
    }))
}
