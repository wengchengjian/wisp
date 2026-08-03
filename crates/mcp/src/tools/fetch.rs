//! MCP fetch_page 工具。

use super::types::{FetchPageArgs, FetchPageResult, ToolContext};
use wisp_core::error::Result;
use wisp_core::{FetchMode, Request};
use wreq_util::Profile;

fn profile_from_name(name: &str) -> Profile {
    match name {
        "firefox" => Profile::Firefox128,
        "safari" => Profile::Safari18,
        _ => Profile::Chrome136,
    }
}

/// 抓取单个网页，返回 HTML 文本。
pub async fn fetch_page(args: FetchPageArgs, ctx: &ToolContext<'_>) -> Result<FetchPageResult> {
    wisp_core::utils::validate_url(&args.url)?;

    let emulation = args.emulation.as_deref().map(profile_from_name);
    let resp = ctx
        .fetch_client
        .fetch_with(
            &Request::get(&args.url),
            FetchMode::Http,
            &wisp_fetcher::FetchOptions { emulation },
        )
        .await?;
    let html = resp.text()?;

    Ok(FetchPageResult {
        url: args.url,
        status: resp.status,
        html,
        bytes: resp.body.len(),
    })
}
