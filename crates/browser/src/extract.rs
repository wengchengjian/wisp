//! 浏览器页面响应提取。

use crate::page::Page;
use wisp_core::error::Result;
use wisp_core::{Request, Response};

/// 从浏览器页面提取统一 Response。
///
/// ARCH: 复用 Page 的高层接口，不再直接执行 document/JS 提取。
pub async fn extract_browser_response(
    page: &Page,
    req: &Request,
    nav_status: u16,
) -> Result<Response> {
    let html = page.content().await?;
    let title = page.title().await?;
    let final_url = page.url().await?;
    let cookies = page.cookie_strings(&req.url).await?;

    Ok(Response::from_browser(
        nav_status,
        final_url,
        html,
        title,
        cookies,
        req.clone(),
    ))
}
