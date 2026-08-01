//! Engine 子模块：fetch page transport。

mod auto_mode;
#[cfg(feature = "browser")]
mod browser;
mod http;

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::auto;
use wisp_core::error::Result;
use wisp_core::{Request, Response};
use wisp_fetcher::FetchMode;
use wisp_http::Client;

#[doc(hidden)]
pub async fn fetch_page(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    mode: FetchMode,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
    cf_domain_locks: &dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
) -> Result<Response> {
    if let Some(override_mode) = req.fetch_mode_override {
        if override_mode == FetchMode::Stealth {
            return auto_mode::fetch_stealth_override(
                fetch_client,
                req,
                proxy_url,
                proxy_clients,
                cf_domain_locks,
            )
            .await;
        }
        return fetch_page_inner(fetch_client, req, proxy_url, override_mode, proxy_clients).await;
    }
    if mode == FetchMode::Auto {
        return auto_mode::fetch_auto(fetch_client, req, proxy_url, rule_engine, proxy_clients)
            .await;
    }
    fetch_page_inner(fetch_client, req, proxy_url, mode, proxy_clients).await
}

#[doc(hidden)]
pub async fn fetch_page_inner(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    mode: FetchMode,
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
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

    http::fetch_http_response(fetch_client, req, proxy_url, proxy_clients).await
}
