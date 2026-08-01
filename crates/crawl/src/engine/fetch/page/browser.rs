//! 浏览器模式抓取。

use wisp_core::error::Result;
use wisp_core::{Request, Response};
use wisp_fetcher::FetchMode;

pub(super) async fn fetch_browser_via_strategy(
    fetch_client: &wisp_fetcher::FetchClient,
    fetch_req: &wisp_fetcher::Request,
    mode: FetchMode,
) -> Result<Response> {
    match mode {
        FetchMode::Dynamic => {
            let strategy = wisp_fetcher::DynamicStrategy::from_config(fetch_client.config());
            fetch_client.fetch_browser(fetch_req, &strategy).await
        }
        FetchMode::Stealth => {
            #[cfg(feature = "stealth")]
            {
                let config = fetch_client.config();
                let cf_jar = std::sync::Arc::new(wisp_fetcher::cookie::CfCookieJar::new(
                    &config.cf_data_dir,
                    config.cf_cookie_ttl,
                ));
                let strategy = wisp_fetcher::StealthStrategy::from_config(config, cf_jar);
                fetch_client.fetch_browser(fetch_req, &strategy).await
            }
            #[cfg(not(feature = "stealth"))]
            {
                Err(wisp_core::error::WispError::Config(
                    "Stealth mode requires 'stealth' feature".into(),
                ))
            }
        }
        _ => unreachable!("上方 if 已过滤非浏览器模式"),
    }
}

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
    let resp = fetch_browser_via_strategy(fetch_client, &fetch_req, mode).await?;
    Ok(to_crawl_response(resp, req, mode))
}
