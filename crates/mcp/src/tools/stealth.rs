//! MCP stealth_fetch 工具：复用共享 FetchClient + StealthStrategy。

#[cfg(feature = "stealth")]
use super::types::{StealthFetchArgs, StealthFetchResult, ToolContext};
#[cfg(feature = "stealth")]
use wisp_core::error::Result;
#[cfg(feature = "stealth")]
use wisp_core::{FetchMode, Request};

#[cfg(feature = "stealth")]
pub async fn stealth_fetch(
    args: StealthFetchArgs,
    ctx: &ToolContext<'_>,
) -> Result<StealthFetchResult> {
    wisp_core::utils::validate_url(&args.url)?;
    let resp = ctx
        .fetch_client
        .fetch(&Request::get(&args.url), FetchMode::Stealth)
        .await?;
    let html = String::from_utf8_lossy(&resp.body).to_string();
    Ok(StealthFetchResult {
        url: resp.url,
        title: resp.title,
        html,
        bytes: resp.body.len(),
    })
}
