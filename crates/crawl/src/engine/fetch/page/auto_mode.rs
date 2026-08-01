//! Auto/Stealth 模式分发。

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::auto;
use wisp_core::error::Result;
use wisp_core::{Request, Response};
use wisp_fetcher::FetchMode;
use wisp_http::Client;

/// 尝试用 cookie jar 中已有的 CF cookie 走 HTTP 抓取（快速路径）。
///
/// 返回 `Ok(Some(resp))`：cookie 存在且 HTTP 成功未被拦截；
/// 返回 `Ok(None)`：无 cookie 或被拦截，调用方需回退 Stealth。
async fn try_http_with_cf_cookie(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
) -> Result<Option<Response>> {
    let Some(url_parsed) = url::Url::parse(&req.url).ok() else {
        return Ok(None);
    };
    let Some(cookie_header) = fetch_client.cookie_jar().header(&url_parsed).await else {
        return Ok(None);
    };
    let mut http_req = req.clone();
    http_req.headers.insert("Cookie".to_string(), cookie_header);
    // CF cookie 与浏览器实际 UA 绑定；快速路径需保持一致性，否则可能被 CF 拒绝。
    if let Some(ua) = fetch_client.cookie_jar().ua(&url_parsed).await {
        http_req.headers.insert("User-Agent".to_string(), ua);
    }
    http_req.fetch_mode_override = Some(FetchMode::Http);
    let resp = super::fetch_page_inner(
        fetch_client,
        &http_req,
        proxy_url,
        FetchMode::Http,
        proxy_clients,
    )
    .await?;
    if resp.status == 200 && auto::blocked_reason(resp.status, &resp.body, &resp.headers).is_none()
    {
        let mut final_resp = resp;
        final_resp.request.fetch_mode_override = Some(FetchMode::Http);
        Ok(Some(final_resp))
    } else {
        Ok(None)
    }
}

pub(super) async fn fetch_stealth_override(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
    cf_domain_locks: &dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
) -> Result<Response> {
    let domain = url::Url::parse(&req.url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if let Some(resp) = try_http_with_cf_cookie(fetch_client, req, proxy_url, proxy_clients).await?
    {
        tracing::info!(
            "AutoMode: 域名锁双重检测 - cookie已存在，HTTP成功 (status={})",
            resp.status
        );
        return Ok(resp);
    }
    let lock = cf_domain_locks
        .entry(domain)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;
    if let Some(resp) = try_http_with_cf_cookie(fetch_client, req, proxy_url, proxy_clients).await?
    {
        tracing::info!(
            "AutoMode: 域名锁等待后 - HTTP+cookie 成功 (status={})",
            resp.status
        );
        return Ok(resp);
    }
    super::fetch_page_inner(
        fetch_client,
        req,
        proxy_url,
        FetchMode::Stealth,
        proxy_clients,
    )
    .await
}

/// Auto 模式：先尝试 HTTP+cookie，再查规则缓存，最后 HTTP 先行等待中间件升级。
pub(super) async fn fetch_auto(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
) -> Result<Response> {
    if let Some(resp) = try_http_with_cf_cookie(fetch_client, req, proxy_url, proxy_clients).await?
    {
        tracing::info!(
            "AutoMode: HTTP+cookie 成功 (status={}), 跳过浏览器",
            resp.status
        );
        return Ok(resp);
    }
    if let Some(cached_mode) = rule_engine.lock().await.resolve(&req.url) {
        return super::fetch_page_inner(fetch_client, req, proxy_url, cached_mode, proxy_clients)
            .await;
    }
    super::fetch_page_inner(fetch_client, req, proxy_url, FetchMode::Http, proxy_clients).await
}
