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
    let resp = fetch_client.fetch(req, mode).await?;
    Ok(to_crawl_response(resp, req, mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_fetcher::FetchClientConfig;

    #[tokio::test]
    async fn browser_fetch_preserves_per_request_proxy_guard() {
        let client = wisp_fetcher::FetchClient::new(FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        })
        .expect("build fetch client");
        let req = Request::get("https://example.com/").with_proxy("http://127.0.0.1:8080");
        let err = fetch_browser_response(&client, &req, FetchMode::Dynamic)
            .await
            .expect_err("per-request proxy 应被浏览器模式拒绝");
        assert!(
            err.to_string().contains("per-request proxy"),
            "错误应说明 per-request proxy: {err}"
        );
    }
}
