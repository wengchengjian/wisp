//! MCP stealth_fetch 工具：复用共享 FetchClient + StealthStrategy。

#[cfg(feature = "stealth")]
use super::fetch_html::fetch_html;
#[cfg(feature = "stealth")]
use super::types::{StealthFetchArgs, StealthFetchResult, ToolContext};
#[cfg(feature = "stealth")]
use wisp_core::FetchMode;
#[cfg(feature = "stealth")]
use wisp_core::error::Result;

#[cfg(feature = "stealth")]
pub async fn stealth_fetch(
    args: StealthFetchArgs,
    ctx: &ToolContext<'_>,
) -> Result<StealthFetchResult> {
    let page = fetch_html(
        ctx,
        &args.url,
        FetchMode::Stealth,
        &wisp_fetcher::FetchOptions::default(),
    )
    .await?;
    Ok(StealthFetchResult {
        url: page.url,
        title: page.title,
        html: page.html,
        bytes: page.bytes,
    })
}
