//! MCP fetch_page 工具。

use super::fetch_html::fetch_html;
use super::types::{FetchPageArgs, FetchPageResult, ToolContext};
use wisp_core::FetchMode;
use wisp_core::error::Result;
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
    let emulation = args.emulation.as_deref().map(profile_from_name);
    let page = fetch_html(
        ctx,
        &args.url,
        FetchMode::Http,
        &wisp_fetcher::FetchOptions { emulation },
    )
    .await?;

    Ok(FetchPageResult {
        url: page.url,
        status: page.status,
        html: page.html,
        bytes: page.bytes,
    })
}
