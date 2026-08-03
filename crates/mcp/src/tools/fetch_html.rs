//! 共享的 MCP 单页抓取 + HTML 解码模块。

use super::types::ToolContext;
use wisp_core::error::Result;
use wisp_core::{FetchMode, Request};

/// 抓取并解码后的单页结果。
pub(crate) struct FetchedHtml {
    pub url: String,
    pub status: u16,
    pub title: Option<String>,
    pub html: String,
    pub bytes: usize,
}

/// 校验 URL、按模式抓取并统一解码 HTML。
pub(crate) async fn fetch_html(
    ctx: &ToolContext<'_>,
    url: &str,
    mode: FetchMode,
    options: &wisp_fetcher::FetchOptions,
) -> Result<FetchedHtml> {
    wisp_core::utils::validate_url(url)?;
    let resp = ctx
        .fetch_client
        .fetch_with(&Request::get(url), mode, options)
        .await?;
    let html = resp.text()?;
    Ok(FetchedHtml {
        url: resp.url,
        status: resp.status,
        title: resp.title,
        html,
        bytes: resp.body.len(),
    })
}
