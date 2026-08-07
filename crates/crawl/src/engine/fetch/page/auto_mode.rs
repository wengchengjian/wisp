//! Auto/Stealth 模式分发。

use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

use crate::CrawlRequest;
use crate::auto;
use wisp_core::Response;
use wisp_core::error::Result;
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
    req: &CrawlRequest,
) -> Result<Option<Response>> {
    let Some(resp) = fetch_client
        .try_http_with_session_cookie(&req.request)
        .await?
    else {
        return Ok(None);
    };
    if auto::blocked_reason(resp.status, &resp.body, &resp.headers).is_none() {
        Ok(Some(resp))
    } else {
        Ok(None)
    }
}

pub(super) async fn fetch_stealth_override(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &CrawlRequest,
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

/// Auto 模式：先尝试 HTTP+cookie，再查规则缓存，最后按需走浏览器挑战。
///
/// 首次访问（无 cookie 也无规则缓存）**直接走 Stealth**，而非先发裸 HTTP 再等
/// StealthUpgrade 升级：裸 HTTP 的 wreq TLS/HTTP2 指纹会被 CF 标记为自动化请求，
/// 污染本 IP/UA 的信任评分（HTTP 前置请求污染），导致后续 Stealth 挑战难以通过。
/// 直接走 Stealth 通过后 cookie 写入共享 seam，后续请求命中 `try_http_with_cf_cookie`
/// 即可降级回 HTTP 复用。
pub(super) async fn fetch_auto(
    fetch_client: &wisp_fetcher::FetchClient,
    req: &CrawlRequest,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
) -> Result<Response> {
    // 1. 快速路径：已有 CF 会话 cookie → HTTP 复用（降级）。
    if let Some(resp) = try_http_with_cf_cookie(fetch_client, req).await? {
        tracing::info!(
            "AutoMode: HTTP+cookie 成功 (status={}), 跳过浏览器",
            resp.status
        );
        return Ok(resp);
    }
    // 2. 规则缓存：该 URL 已学习明确模式 → 直接按该模式抓取。
    if let Some(cached_mode) = rule_engine.lock().await.resolve(&req.url) {
        return super::fetch_page_inner(fetch_client, req, cached_mode).await;
    }
    // 3. 首次访问：直接走 Stealth，跳过 HTTP 前置请求，避免污染 CF 信任评分。
    super::fetch_page_inner(fetch_client, req, FetchMode::Stealth).await
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
