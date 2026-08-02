//! Auto/Stealth 模式分发。

use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

use crate::auto;
use wisp_core::error::Result;
use wisp_core::{Request, Response};
use wisp_fetcher::FetchMode;

const CF_DOMAIN_LOCK_LIMIT: usize = 1024;
static GLOBAL_CF_LOCK: LazyLock<Arc<Mutex<()>>> = LazyLock::new(|| Arc::new(Mutex::new(())));
static CF_LOCK_LIMIT_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn cf_domain_lock(
    locks: &dashmap::DashMap<String, Arc<Mutex<()>>>,
    domain: &str,
) -> Arc<Mutex<()>> {
    if locks.len() < CF_DOMAIN_LOCK_LIMIT {
        return locks
            .entry(domain.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
    }
    if !CF_LOCK_LIMIT_WARNED.swap(true, Ordering::SeqCst) {
        tracing::warn!("cf_domain_locks 达到上限 {CF_DOMAIN_LOCK_LIMIT}，回退全局锁");
    }
    GLOBAL_CF_LOCK.clone()
}

/// 尝试用共享 cookie seam 中已有的 CF cookie 走 HTTP 抓取（快速路径）。
async fn try_http_with_cf_cookie(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
) -> Result<Option<Response>> {
    let Some(resp) = fetch_client.fetch_http_with_cf_cookie(req).await? else {
        return Ok(None);
    };
    if resp.status == 200 && auto::blocked_reason(resp.status, &resp.body, &resp.headers).is_none()
    {
        Ok(Some(resp))
    } else {
        Ok(None)
    }
}

pub(super) async fn fetch_stealth_override(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    cf_domain_locks: &dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
) -> Result<Response> {
    let domain = url::Url::parse(&req.url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if let Some(resp) = try_http_with_cf_cookie(fetch_client, req).await? {
        tracing::info!(
            "AutoMode: 域名锁双重检测 - cookie已存在，HTTP成功 (status={})",
            resp.status
        );
        return Ok(resp);
    }
    let lock = cf_domain_lock(cf_domain_locks, &domain);
    let _guard = lock.lock().await;
    if let Some(resp) = try_http_with_cf_cookie(fetch_client, req).await? {
        tracing::info!(
            "AutoMode: 域名锁等待后 - HTTP+cookie 成功 (status={})",
            resp.status
        );
        return Ok(resp);
    }
    super::fetch_page_inner(fetch_client, req, FetchMode::Stealth).await
}

/// Auto 模式：先尝试 HTTP+cookie，再查规则缓存，最后 HTTP 先行等待中间件升级。
pub(super) async fn fetch_auto(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &Request,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
) -> Result<Response> {
    if let Some(resp) = try_http_with_cf_cookie(fetch_client, req).await? {
        tracing::info!(
            "AutoMode: HTTP+cookie 成功 (status={}), 跳过浏览器",
            resp.status
        );
        return Ok(resp);
    }
    if let Some(cached_mode) = rule_engine.lock().await.resolve(&req.url) {
        return super::fetch_page_inner(fetch_client, req, cached_mode).await;
    }
    super::fetch_page_inner(fetch_client, req, FetchMode::Http).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashmap::DashMap;

    #[test]
    fn domain_lock_inserts_below_limit() {
        let locks = DashMap::new();
        let lock = cf_domain_lock(&locks, "a.example.com");
        assert_eq!(locks.len(), 1);
        let _guard = lock.blocking_lock();
    }

    #[test]
    fn domain_lock_falls_back_at_limit() {
        let locks = DashMap::new();
        for i in 0..CF_DOMAIN_LOCK_LIMIT {
            locks.insert(format!("d{i}.example.com"), Arc::new(Mutex::new(())));
        }
        let lock = cf_domain_lock(&locks, "new.example.com");
        assert_eq!(locks.len(), CF_DOMAIN_LOCK_LIMIT);
        assert!(locks.get("new.example.com").is_none());
        let _guard = lock.blocking_lock();
    }
}
