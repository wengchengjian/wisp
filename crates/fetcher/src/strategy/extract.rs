//! 浏览器页面响应提取。

use wisp_browser::Page;
use wisp_core::error::Result;
use wisp_core::{Request, Response};

/// 从浏览器页面提取统一 Response。
///
/// ARCH: 从 FetchClient::extract_browser_response 提取为公共 helper，
/// 供 DynamicStrategy / StealthStrategy 复用。
pub(crate) async fn extract_browser_response(
    page: &Page,
    req: &Request,
    nav_status: u16,
) -> Result<Response> {
    let html = page
        .evaluate_as_string("document.documentElement.outerHTML")
        .await?;
    let title = page.evaluate_as_string("document.title").await?;
    let final_url = page.evaluate_as_string("window.location.href").await?;

    let cookies_raw = page
        .evaluate_as_string("(() => { try { return document.cookie; } catch { return ''; } })()")
        .await?;
    let cookies: Vec<String> = cookies_raw
        .split(';')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();

    Ok(Response::from_browser(
        nav_status,
        final_url,
        html,
        title,
        cookies,
        req.clone(),
    ))
}
