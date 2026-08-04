//! MCP fetch_page 工具。

use super::fetch_html::fetch_html;
use super::types::ToolContext;
use crate::protocol::{Tool, TypedRun};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use wisp_core::FetchMode;
use wisp_core::error::Result;
use wreq_util::Profile;

/// `fetch_page` arguments.
#[derive(Debug, Deserialize)]
pub struct FetchPageArgs {
    /// Target URL.
    pub url: String,
    /// Optional TLS fingerprint profile name.
    #[serde(default)]
    pub emulation: Option<String>,
}

/// `fetch_page` result.
#[derive(Debug, Serialize)]
pub struct FetchPageResult {
    /// Final URL.
    pub url: String,
    /// HTTP status code.
    pub status: u16,
    /// Decoded HTML.
    pub html: String,
    /// Raw body byte count.
    pub bytes: usize,
}

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

fn fetch_page_run<'a>(
    args: FetchPageArgs,
    ctx: &'a ToolContext<'a>,
) -> Pin<Box<dyn Future<Output = Result<FetchPageResult>> + Send + 'a>> {
    Box::pin(fetch_page(args, ctx))
}

pub(crate) fn spec() -> Tool {
    Tool::from_handler(
        "fetch_page",
        "抓取单个网页，返回 HTML 文本。支持 wreq TLS 指纹模拟绕过轻度反 bot。",
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "目标 URL" },
                "emulation": {
                    "type": "string",
                    "enum": ["chrome", "firefox", "safari"],
                    "description": "浏览器指纹模拟，默认 chrome"
                }
            },
            "required": ["url"]
        }),
        fetch_page_run as TypedRun<FetchPageArgs, FetchPageResult>,
    )
}
