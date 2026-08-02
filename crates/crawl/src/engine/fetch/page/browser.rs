//! 浏览器模式抓取。

use wisp_core::error::Result;
use wisp_core::{Request, Response};
use wisp_fetcher::FetchMode;

fn to_crawl_response(resp: Response, req: &Request, mode: FetchMode) -> Response {
    let content_type = resp
        .headers
        .get("content-type")
        .or_else(|| resp.headers.get("Content-Type"))
        .cloned()
        .unwrap_or_default();
    let mut final_req = req.clone();
    if final_req.fetch_mode_override.is_none() {
        final_req.fetch_mode_override = Some(mode);
    }
    Response::from_parts(
        resp.status,
        resp.url.clone(),
        resp.headers.clone(),
        resp.body.clone(),
        resp.title.clone(),
        resp.cookies.clone(),
        final_req,
        content_type,
        false,
    )
}

pub(super) async fn fetch_browser_response(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    mode: FetchMode,
) -> Result<Response> {
    let fetch_req = wisp_fetcher::Request {
        url: req.url.clone(),
        method: req.method,
        headers: req.headers.clone(),
        body: req.body.clone(),
        ..Default::default()
    };
    let resp = fetch_client.fetch(&fetch_req, mode).await?;
    Ok(to_crawl_response(resp, req, mode))
}
