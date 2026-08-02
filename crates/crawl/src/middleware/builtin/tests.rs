//! builtin middleware tests.

use super::super::ItemPipeline;
use super::super::pipeline::FilterFieldsPipeline;
use super::*;
use crate::{Request, Response};
use wisp_core::error::WispError;
use wisp_fetcher::FetchMode;

use crate::auto::ModeRuleEngine;
use crate::middleware::MiddlewareChain;
use crate::runtime::robots::RobotsCache;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use wisp_proxy::{ProxyPool, RotationStrategy};

fn make_req() -> Request {
    Request {
        url: "http://example.com".into(),
        method: crate::Method::Get,
        headers: HashMap::new(),
        body: None,
        meta: Value::Null,
        callback: None,
        spider: None,
        priority: 0,
        depth: 0,
        proxy: None,
        fetch_mode_override: None,
        retry_count: 0,
    }
}

fn make_ctx() -> CrawlContext {
    CrawlContext {
        spider_name: "test".into(),
        fetch_mode: FetchMode::Http,
        max_concurrent: 8,
        max_pages: 1000,
        obey_robots: false,
        pages_crawled: 0,
        errors: 0,
    }
}

#[tokio::test]
async fn test_ua_rotation_middleware() {
    let mw = UaRotationMiddleware::desktop();
    let ctx = make_ctx();
    let mut req = make_req();
    let action = mw.process_request(&mut req, &ctx).await;
    assert_eq!(action, RequestMwAction::Modified);
    assert!(req.headers.contains_key("User-Agent"));
}

#[tokio::test]
async fn test_headers_middleware() {
    let mw = HeadersMiddleware::new(vec![("X-Custom".into(), "value1".into())]);
    let ctx = make_ctx();
    let mut req = make_req();
    let action = mw.process_request(&mut req, &ctx).await;
    assert_eq!(action, RequestMwAction::Modified);
    assert_eq!(req.headers.get("X-Custom").unwrap(), "value1");
}

#[tokio::test]
async fn test_retry_middleware_always_retries_fetch_errors() {
    // RetryMiddleware 只决定"是否值得重试"，不维护计数
    // fetch_page 返回 Err 都是网络层错误，默认全部可重试
    let mw = RetryMiddleware::new(Duration::ZERO);
    let ctx = make_ctx();

    // 各种网络错误都应可重试
    for err in &[
        "operation timed out",
        "connection reset by peer",
        "broken pipe",
        "connection refused",
        "dns resolution failed",
        "Connection failed to 127.0.0.1: error sending request",
        "tls handshake error",
    ] {
        let req = make_req();
        let wisp_err = WispError::Timeout(err.to_string());
        let action = mw.process_error(&req, &wisp_err, &ctx).await;
        assert_eq!(action, ErrorAction::Retry, "网络错误 '{}' 应可重试", err);
    }
}

#[tokio::test]
async fn test_cache_middleware() {
    let mw = CacheMiddleware::new(
        Arc::new(wisp_storage::MemoryStore::default()),
        Some(Duration::from_secs(60)),
    );
    let ctx = make_ctx();
    let mut req = make_req();
    assert_eq!(
        mw.process_request(&mut req, &ctx).await,
        RequestMwAction::Continue
    );

    let mut resp = Response::from_http(
        200,
        "http://example.com".into(),
        HashMap::new(),
        b"hello".to_vec(),
        String::new(),
        req.clone(),
    );
    mw.process_response(&mut resp, &ctx).await;

    let mut req2 = make_req();
    match mw.process_request(&mut req2, &ctx).await {
        RequestMwAction::Respond(cached) => {
            assert_eq!(cached.status, 200);
            assert!(cached.from_cache);
        }
        other => panic!("expected Respond, got {:?}", other),
    }
}

#[tokio::test]
async fn test_delay_middleware() {
    let mw = DelayMiddleware::from_millis(10);
    let ctx = make_ctx();
    let mut req = make_req();
    let start = std::time::Instant::now();
    let action = mw.process_request(&mut req, &ctx).await;
    assert_eq!(action, RequestMwAction::Continue);
    assert!(start.elapsed() >= Duration::from_millis(10));
}

#[tokio::test]
async fn test_filter_fields_pipeline() {
    let pipeline = FilterFieldsPipeline::new(vec!["title", "url"]);
    let item = serde_json::json!({"title": "Hello", "url": "http://x.com", "extra": 123});
    let result = pipeline.process_item(item, &make_ctx()).await.unwrap();
    assert_eq!(result["title"], "Hello");
    assert!(result.get("extra").is_none());
}

#[tokio::test]
async fn test_priority_ordering() {
    let cache = CacheMiddleware::new(
        Arc::new(wisp_storage::MemoryStore::default()),
        Some(Duration::from_secs(1)),
    );
    let headers = HeadersMiddleware::new(vec![]);
    let delay = DelayMiddleware::from_millis(0);
    let ua = UaRotationMiddleware::desktop();

    assert_eq!(cache.priority(), 3);
    assert_eq!(headers.priority(), 10);
    assert_eq!(delay.priority(), 15);
    assert_eq!(ua.priority(), 20);
}

// === DynamicUpgradeMiddleware 测试 ===

fn make_resp(status: u16, body: &[u8]) -> Response {
    Response::from_parts(
        status,
        "http://example.com".into(),
        HashMap::new(),
        body.to_vec(),
        None,
        Vec::new(),
        make_req(),
        String::new(),
        false,
    )
}

#[tokio::test]
async fn dynamic_upgrade_triggers_for_spa_body() {
    let mw = DynamicUpgradeMiddleware::new();
    let ctx = make_ctx();
    let mut resp = make_resp(
        200,
        b"<html><script id=\"__NUXT_DATA__\">{}</script></html>",
    );
    let action = mw.process_response(&mut resp, &ctx).await;
    match action {
        ResponseMwAction::Refetch(req) => {
            assert_eq!(req.fetch_mode_override, Some(FetchMode::Dynamic));
        }
        other => panic!("expected Refetch, got {:?}", other),
    }
}

#[tokio::test]
async fn dynamic_upgrade_triggers_for_dom_mutation() {
    let mw = DynamicUpgradeMiddleware::new();
    let ctx = make_ctx();
    let mut resp = make_resp(200, b"<script>el.innerHTML = 'loaded'</script>");
    let action = mw.process_response(&mut resp, &ctx).await;
    match action {
        ResponseMwAction::Refetch(req) => {
            assert_eq!(req.fetch_mode_override, Some(FetchMode::Dynamic));
        }
        other => panic!("expected Refetch, got {:?}", other),
    }
}

#[tokio::test]
async fn dynamic_upgrade_skips_when_override_already_set() {
    let mw = DynamicUpgradeMiddleware::new();
    let ctx = make_ctx();
    let mut resp = make_resp(200, b"<script id=\"__NUXT_DATA__\">{}</script>");
    resp.request.fetch_mode_override = Some(FetchMode::Dynamic);
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(action, ResponseMwAction::Continue);
}

// === StealthUpgradeMiddleware 快速短路测试 ===

fn stealth_mw() -> StealthUpgradeMiddleware {
    StealthUpgradeMiddleware::new(Arc::new(Mutex::new(ModeRuleEngine::new())))
}

#[tokio::test]
async fn stealth_upgrade_skips_non_html_content_type() {
    let mw = stealth_mw();
    let ctx = make_ctx();
    let mut resp = make_resp(200, b"<title>Just a moment...</title>");
    resp.content_type = "application/json".into();
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(
        action,
        ResponseMwAction::Continue,
        "非 HTML 不应触发 body 扫描"
    );
}

#[tokio::test]
async fn stealth_upgrade_skips_explicit_override() {
    let mw = stealth_mw();
    let ctx = make_ctx();
    let mut resp = make_resp(200, b"<title>Just a moment...</title>");
    resp.request.fetch_mode_override = Some(FetchMode::Dynamic);
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(
        action,
        ResponseMwAction::Continue,
        "已显式指定模式不应再升级"
    );
}

#[tokio::test]
async fn stealth_upgrade_still_detects_html_blocked() {
    let mw = stealth_mw();
    let ctx = make_ctx();
    let mut resp = make_resp(200, b"<title>Just a moment...</title>");
    resp.content_type = "text/html; charset=utf-8".into();
    let action = mw.process_response(&mut resp, &ctx).await;
    assert!(
        matches!(action, ResponseMwAction::Refetch(_)),
        "HTML 挑战页仍应升级"
    );
}

#[tokio::test]
async fn dynamic_upgrade_skips_normal_html() {
    let mw = DynamicUpgradeMiddleware::new();
    let ctx = make_ctx();
    let mut resp = make_resp(
        200,
        b"<html><body><h1>Hello</h1><p>Content</p></body></html>",
    );
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(action, ResponseMwAction::Continue);
}

#[tokio::test]
async fn dynamic_upgrade_skips_non_200() {
    let mw = DynamicUpgradeMiddleware::new();
    let ctx = make_ctx();
    let mut resp = make_resp(403, b"<script id=\"__NUXT_DATA__\">{}</script>");
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(action, ResponseMwAction::Continue);
}

#[tokio::test]
async fn dynamic_upgrade_triggers_for_high_script_density() {
    let mw = DynamicUpgradeMiddleware::new();
    let ctx = make_ctx();
    // 6 个 <script> 标签（阈值），无 SPA 标识、无 DOM 修改方法
    let body = b"<html><head>\
<script src='/a.js'></script>\
<script src='/b.js'></script>\
<script src='/c.js'></script>\
<script src='/d.js'></script>\
<script src='/e.js'></script>\
<script src='/f.js'></script>\
</head><body>ok</body></html>";
    let mut resp = make_resp(200, body);
    let action = mw.process_response(&mut resp, &ctx).await;
    match action {
        ResponseMwAction::Refetch(req) => {
            assert_eq!(req.fetch_mode_override, Some(FetchMode::Dynamic));
        }
        other => panic!("expected Refetch, got {:?}", other),
    }
}

#[tokio::test]
async fn dynamic_upgrade_skips_low_script_density() {
    let mw = DynamicUpgradeMiddleware::new();
    let ctx = make_ctx();
    // 3 个 <script> 标签（低于阈值 6）
    let body = b"<html><head>\
<script src='/a.js'></script>\
<script src='/b.js'></script>\
<script src='/c.js'></script>\
</head><body>ok</body></html>";
    let mut resp = make_resp(200, body);
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(action, ResponseMwAction::Continue);
}

// === BlockedRetryMiddleware 测试 ===

#[tokio::test]
async fn blocked_retry_triggers_for_403_without_override() {
    // 无 fetch_mode_override 时，403 应触发 Refetch（HTTP 模式被拦截，期望重试）
    let mw = BlockedRetryMiddleware::new(Duration::ZERO);
    let ctx = make_ctx();
    let mut resp = make_resp(403, b"");
    let action = mw.process_response(&mut resp, &ctx).await;
    match action {
        ResponseMwAction::Refetch(_) => {}
        other => panic!("expected Refetch, got {:?}", other),
    }
}

#[tokio::test]
async fn blocked_retry_skips_for_stealth_override() {
    // Stealth 模式下不 Refetch：浏览器已是最强模式，403 通常是挑战前的状态码，
    // Refetch 只会再次触发挑战流程，无法突破拦截。
    let mw = BlockedRetryMiddleware::new(Duration::ZERO);
    let ctx = make_ctx();
    let mut resp = make_resp(403, b"");
    resp.request.fetch_mode_override = Some(FetchMode::Stealth);
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(action, ResponseMwAction::Continue);
}

#[tokio::test]
async fn blocked_retry_skips_for_200() {
    // 200 正常响应不触发 Refetch
    let mw = BlockedRetryMiddleware::new(Duration::ZERO);
    let ctx = make_ctx();
    let mut resp = make_resp(200, b"<html>ok</html>");
    let action = mw.process_response(&mut resp, &ctx).await;
    assert_eq!(action, ResponseMwAction::Continue);
}

// === default_middlewares 分类逻辑测试 ===

/// 构造测试用共享 FetchClient。
fn make_fetch_client() -> Arc<wisp_fetcher::FetchClient> {
    Arc::new(
        wisp_fetcher::FetchClient::new(wisp_fetcher::FetchClientConfig::default())
            .expect("build fetch client"),
    )
}

/// 验证默认中间件按 fetch_mode + 配置正确分类注入。
#[test]
fn default_middlewares_classifies_by_mode_and_config() {
    let http_client = make_fetch_client();
    let robots_cache = Arc::new(RobotsCache::new());
    let rule_engine = Arc::new(Mutex::new(ModeRuleEngine::new()));

    let priorities =
        |mws: &[Arc<dyn Middleware>]| mws.iter().map(|m| m.priority()).collect::<Vec<_>>();

    // Auto + 全配置：应注入完整链（含模式升级类 40/45）
    let auto_p = priorities(&default_middlewares(DefaultMiddlewareConfig {
        fetch_mode: FetchMode::Auto,
        delay: Duration::from_millis(100),
        headers: vec![("Accept".into(), "text/html".into())],
        ua_middleware: Some(Arc::new(UaRotationMiddleware::desktop())),
        cookie_challenge: true,
        dynamic_upgrade: true,
        obey_robots: true,
        cache_store: None,
        http_client: http_client.clone(),
        robots_cache: robots_cache.clone(),
        rule_engine: rule_engine.clone(),
    }));
    assert!(auto_p.contains(&8), "obey_robots=true 应含 Robots");
    assert!(auto_p.contains(&15), "delay>0 应含 Delay");
    assert!(auto_p.contains(&20), "应含 UaRotation");
    assert!(auto_p.contains(&10), "headers 非空应含 Headers");
    assert!(auto_p.contains(&40), "Auto 应含 DynamicUpgrade");
    assert!(auto_p.contains(&45), "Auto 应含 StealthUpgrade");
    assert!(auto_p.contains(&50), "应含 CookieChallenge");
    assert!(auto_p.contains(&80), "应含 BlockedRetry");
    assert!(auto_p.contains(&90), "应含 Retry");

    // Auto 默认配置：不含 DynamicUpgrade（避免静态站点全量扫描），仍含 StealthUpgrade
    let auto_default_p = priorities(&default_middlewares(DefaultMiddlewareConfig {
        fetch_mode: FetchMode::Auto,
        delay: Duration::ZERO,
        headers: Vec::new(),
        ua_middleware: None,
        cookie_challenge: false,
        dynamic_upgrade: false,
        obey_robots: false,
        cache_store: None,
        http_client: http_client.clone(),
        robots_cache: robots_cache.clone(),
        rule_engine: rule_engine.clone(),
    }));
    assert!(
        !auto_default_p.contains(&40),
        "默认 Auto 不应含 DynamicUpgrade"
    );
    assert!(auto_default_p.contains(&45), "Auto 应含 StealthUpgrade");

    // Http + 最小配置：不应含升级类和条件类
    let http_p = priorities(&default_middlewares(DefaultMiddlewareConfig {
        fetch_mode: FetchMode::Http,
        delay: Duration::ZERO,
        headers: Vec::new(),
        ua_middleware: None,
        cookie_challenge: false,
        dynamic_upgrade: false,
        obey_robots: false,
        cache_store: None,
        http_client: http_client.clone(),
        robots_cache: robots_cache.clone(),
        rule_engine: rule_engine.clone(),
    }));
    assert!(!http_p.contains(&8), "obey_robots=false 不应含 Robots");
    assert!(!http_p.contains(&15), "delay=0 不应含 Delay");
    assert!(!http_p.contains(&20), "未配置 UA 不应含 UaRotation");
    assert!(!http_p.contains(&50), "未启用不应含 CookieChallenge");
    assert!(!http_p.contains(&40), "Http 不应含 DynamicUpgrade");
    assert!(!http_p.contains(&45), "Http 不应含 StealthUpgrade");
    // 总是注入的仍存在
    assert!(http_p.contains(&80), "总是含 BlockedRetry");
    assert!(http_p.contains(&90), "总是含 Retry");

    // Dynamic/Stealth 模式同样不注入升级类
    for mode in [FetchMode::Dynamic, FetchMode::Stealth] {
        let p = priorities(&default_middlewares(DefaultMiddlewareConfig {
            fetch_mode: mode,
            delay: Duration::ZERO,
            headers: Vec::new(),
            ua_middleware: None,
            cookie_challenge: false,
            dynamic_upgrade: false,
            obey_robots: false,
            cache_store: None,
            http_client: http_client.clone(),
            robots_cache: robots_cache.clone(),
            rule_engine: rule_engine.clone(),
        }));
        assert!(!p.contains(&40), "{:?} 不应含 DynamicUpgrade", mode);
        assert!(!p.contains(&45), "{:?} 不应含 StealthUpgrade", mode);
    }
}

/// 验证默认链的实际名称，便于 profiling 时核对中间件清单。
#[test]
fn default_middlewares_names_match_config() {
    let http_client = make_fetch_client();
    let robots_cache = Arc::new(RobotsCache::new());
    let rule_engine = Arc::new(Mutex::new(ModeRuleEngine::new()));
    let mws = default_middlewares(DefaultMiddlewareConfig {
        fetch_mode: FetchMode::Auto,
        delay: Duration::ZERO,
        headers: vec![("Accept".into(), "text/html".into())],
        ua_middleware: Some(Arc::new(UaRotationMiddleware::desktop())),
        cookie_challenge: true,
        dynamic_upgrade: false,
        obey_robots: false,
        cache_store: None,
        http_client,
        robots_cache,
        rule_engine,
    });
    let chain = MiddlewareChain {
        middlewares: mws,
        pipelines: Vec::new(),
    };
    let names = chain.names();
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("StealthUpgradeMiddleware")),
        "Auto 应含 StealthUpgrade: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("CookieChallengeMiddleware")),
        "应含 CookieChallenge: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("BlockedRetryMiddleware")),
        "应含 BlockedRetry: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("RetryMiddleware")),
        "应含 Retry: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.ends_with("DynamicUpgradeMiddleware")),
        "默认不应含 DynamicUpgrade: {names:?}"
    );
}

#[tokio::test]
async fn test_proxy_injection_middleware() {
    let pool = Arc::new(ProxyPool::new(
        vec!["http://p1:8080".into(), "http://p2:8080".into()],
        RotationStrategy::Sequential,
    ));
    let mw = ProxyInjectionMiddleware::new(pool);
    let ctx = make_ctx();

    let mut req = make_req();
    assert_eq!(
        mw.process_request(&mut req, &ctx).await,
        RequestMwAction::Modified
    );
    assert_eq!(req.proxy.as_deref(), Some("http://p1:8080"));

    let mut req2 = make_req();
    mw.process_request(&mut req2, &ctx).await;
    assert_eq!(req2.proxy.as_deref(), Some("http://p2:8080"));
}
