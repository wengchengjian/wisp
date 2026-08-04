//! MCP stealth_fetch 工具：复用共享 FetchClient + StealthStrategy。

use super::fetch_html::fetch_html;
use super::types::ToolContext;
use crate::protocol::{Tool, TypedRun};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use wisp_core::FetchMode;
use wisp_core::error::Result;

/// `stealth_fetch` arguments.
#[derive(Debug, Deserialize)]
pub struct StealthFetchArgs {
    /// Target URL.
    pub url: String,
}

/// `stealth_fetch` result.
#[derive(Debug, Serialize)]
pub struct StealthFetchResult {
    /// Final URL.
    pub url: String,
    /// Page title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Decoded HTML.
    pub html: String,
    /// Raw body byte count.
    pub bytes: usize,
}

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

fn stealth_fetch_run<'a>(
    args: StealthFetchArgs,
    ctx: &'a ToolContext<'a>,
) -> Pin<Box<dyn Future<Output = Result<StealthFetchResult>> + Send + 'a>> {
    Box::pin(stealth_fetch(args, ctx))
}

pub(crate) fn spec() -> Tool {
    Tool::from_handler(
        "stealth_fetch",
        "浏览器隐身抓取（CF 挑战解决 + 人类行为模拟，复用共享浏览器池）。",
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" }
            },
            "required": ["url"]
        }),
        stealth_fetch_run as TypedRun<StealthFetchArgs, StealthFetchResult>,
    )
}
