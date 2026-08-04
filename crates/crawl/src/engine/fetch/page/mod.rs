//! Engine 子模块：fetch page transport。

mod auto_mode;
#[cfg(feature = "browser")]
mod browser;

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::CrawlRequest;
use crate::auto;
use wisp_core::Response;
use wisp_core::error::Result;
use wisp_fetcher::FetchMode;

pub(crate) async fn fetch_page(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &CrawlRequest,
    mode: FetchMode,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
    cf_domain_locks: &dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
) -> Result<Response> {
    let mut resp = if let Some(override_mode) = req.fetch_mode_override {
        if override_mode == FetchMode::Stealth {
            auto_mode::fetch_stealth_override(fetch_client, req, cf_domain_locks).await?
        } else {
            fetch_page_inner(fetch_client, req, override_mode).await?
        }
    } else if mode == FetchMode::Auto {
        auto_mode::fetch_auto(fetch_client, req, rule_engine).await?
    } else {
        fetch_page_inner(fetch_client, req, mode).await?
    };
    resp.request = req.clone();
    Ok(resp)
}

pub(crate) async fn fetch_page_inner(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &CrawlRequest,
    mode: FetchMode,
) -> Result<Response> {
    #[cfg(feature = "browser")]
    if mode == FetchMode::Dynamic || mode == FetchMode::Stealth {
        return browser::fetch_browser_response(fetch_client, req, mode).await;
    }

    #[cfg(not(feature = "browser"))]
    if mode == FetchMode::Dynamic || mode == FetchMode::Stealth {
        return Err(wisp_core::error::WispError::Config(format!(
            "{mode:?} mode requires 'browser' feature"
        )));
    }

    fetch_client.fetch(&req.request, mode).await
}
