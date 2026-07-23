//! 内建中间件实现。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{Middleware, MwAction, ErrorAction, CrawlContext, ItemPipeline};
use crate::crawl::{SpiderRequest, SpiderResponse, Method};
use crate::crawl::auto::{self, ModeRuleEngine};
use crate::crawl::runtime::request_cache::{RequestCache, CachedEntry};
use crate::crawl::runtime::robots::RobotsCache;
use crate::fetcher::FetchMode;
use crate::http::Client;

// === 请求修改类 ===

/// UA 轮换中间件：每次请求随机选择一个 User-Agent。
pub struct UaRotationMiddleware {
    agents: Vec<String>,
    index: std::sync::atomic::AtomicUsize,
}

impl UaRotationMiddleware {
    /// 使用桌面 UA 列表创建（Chrome/Edge 136，匹配默认 TLS 指纹）。
    pub fn desktop() -> Self {
        Self {
            agents: vec![
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".into(),
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0".into(),
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0".into(),
            ],
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// 使用自定义 UA 列表创建。
    pub fn with_agents(agents: Vec<String>) -> Self {
        Self { agents, index: std::sync::atomic::AtomicUsize::new(0) }
    }
}

#[async_trait]
impl Middleware for UaRotationMiddleware {
    fn priority(&self) -> u32 { 20 }

    async fn process_request(&self, req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        if self.agents.is_empty() {
            return MwAction::Continue;
        }
        let idx = self.index.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.agents.len();
        req.headers.insert("User-Agent".to_string(), self.agents[idx].clone());
        MwAction::Modified
    }
}

/// 重试中间件：在错误时决定重试。
pub struct RetryMiddleware {
    max_retries: u32,
    retry_delay: Duration,
}

impl RetryMiddleware {
    pub fn new(max_retries: u32, retry_delay: Duration) -> Self {
        Self { max_retries, retry_delay }
    }
}

#[async_trait]
impl Middleware for RetryMiddleware {
    fn priority(&self) -> u32 { 90 }

    async fn process_error(&self, req: &SpiderRequest, _err: &str, _ctx: &CrawlContext) -> ErrorAction {
        let count = req.meta.get("_retry").and_then(|v| v.as_u64()).unwrap_or(0);
        if count < self.max_retries as u64 {
            if !self.retry_delay.is_zero() {
                tokio::time::sleep(self.retry_delay).await;
            }
            ErrorAction::Retry
        } else {
            ErrorAction::Propagate
        }
    }
}

/// 代理注入中间件：从代理池中为每个请求分配代理。
///
/// 代理由中间件全权管理，引擎仅读取 `req.proxy` 并应用。
pub struct ProxyInjectionMiddleware {
    pool: Arc<crate::proxy::ProxyPool>,
}

impl ProxyInjectionMiddleware {
    pub fn new(pool: Arc<crate::proxy::ProxyPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Middleware for ProxyInjectionMiddleware {
    fn priority(&self) -> u32 { 30 }

    async fn process_request(&self, req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        if let Some(proxy) = self.pool.next() {
            req.proxy = Some(proxy);
            MwAction::Modified
        } else {
            MwAction::Continue
        }
    }
}

/// 请求头注入中间件：为每个请求添加固定 headers。
pub struct HeadersMiddleware {
    headers: Vec<(String, String)>,
}

impl HeadersMiddleware {
    pub fn new(headers: Vec<(String, String)>) -> Self {
        Self { headers }
    }
}

#[async_trait]
impl Middleware for HeadersMiddleware {
    fn priority(&self) -> u32 { 10 }

    async fn process_request(&self, req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        if self.headers.is_empty() {
            return MwAction::Continue;
        }
        for (k, v) in &self.headers {
            req.headers.insert(k.clone(), v.clone());
        }
        MwAction::Modified
    }
}

// === 响应挑战类 ===

/// Cookie 挑战中间件：自动解决多步 Set-Cookie + JS 重定向类反爬。
///
/// 检测特征：403 + Set-Cookie + body 极短（< 200 字节）。
/// 解决方式：累积 cookie 并通过 `MwAction::Refetch` 重新获取。
pub struct CookieChallengeMiddleware {
    /// 最大累积轮数（默认 3）
    max_rounds: usize,
}

impl CookieChallengeMiddleware {
    pub fn new(max_rounds: usize) -> Self {
        Self { max_rounds }
    }
}

impl Default for CookieChallengeMiddleware {
    fn default() -> Self {
        Self { max_rounds: 3 }
    }
}

#[async_trait]
impl Middleware for CookieChallengeMiddleware {
    fn priority(&self) -> u32 { 50 }

    async fn process_response(&self, resp: &mut SpiderResponse, _ctx: &CrawlContext) -> MwAction {
        if resp.status != 403 || resp.body.len() >= 200 {
            return MwAction::Continue;
        }
        let set_cookie = match resp.headers.get("set-cookie") {
            Some(sc) => sc.clone(),
            None => return MwAction::Continue,
        };
        let cookie_pair = set_cookie.split(';').next().unwrap_or("").to_string();
        if cookie_pair.is_empty() {
            return MwAction::Continue;
        }
        let existing = resp.request.headers.get("Cookie").cloned().unwrap_or_default();
        let new_cookie = if existing.is_empty() {
            cookie_pair
        } else {
            if existing.contains(&cookie_pair) {
                return MwAction::Continue;
            }
            format!("{}; {}", existing, cookie_pair)
        };
        let cookie_count = new_cookie.matches("; ").count() + 1;
        if cookie_count > self.max_rounds {
            return MwAction::Continue;
        }
        let mut new_req = resp.request.clone();
        new_req.headers.insert("Cookie".to_string(), new_cookie);
        MwAction::Refetch(new_req)
    }
}

// === 过滤/限制类 ===

/// 域名过滤中间件：仅允许请求访问指定域名，其他域名直接 Skip。
///
/// 空域名集合 = 允许所有域名。
pub struct DomainFilterMiddleware {
    allowed: HashSet<String>,
}

impl DomainFilterMiddleware {
    /// 从域名列表创建（如 `["example.com", "api.example.com"]`）。
    pub fn new(allowed: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self { allowed: allowed.into_iter().map(|s| s.into()).collect() }
    }
}

#[async_trait]
impl Middleware for DomainFilterMiddleware {
    fn priority(&self) -> u32 { 0 }

    async fn process_request(&self, req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        if self.allowed.is_empty() {
            return MwAction::Continue;
        }
        if let Ok(parsed) = url::Url::parse(&req.url) {
            if let Some(host) = parsed.host_str() {
                if !self.allowed.contains(host) {
                    return MwAction::Skip;
                }
            }
        }
        MwAction::Continue
    }
}

/// 深度限制中间件：请求深度超过上限时 Skip。
pub struct DepthLimitMiddleware {
    max_depth: u32,
}

impl DepthLimitMiddleware {
    pub fn new(max_depth: u32) -> Self {
        Self { max_depth }
    }
}

#[async_trait]
impl Middleware for DepthLimitMiddleware {
    fn priority(&self) -> u32 { 5 }

    async fn process_request(&self, req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        if req.depth > self.max_depth {
            MwAction::Skip
        } else {
            MwAction::Continue
        }
    }
}

/// 响应缓存中间件：缓存命中时通过 `MwAction::Respond` 短路，跳过网络请求。
///
/// 响应返回后自动写入缓存。
pub struct CacheMiddleware {
    cache: RequestCache,
}

impl CacheMiddleware {
    pub fn new(cache: RequestCache) -> Self {
        Self { cache }
    }

    /// 便捷构造：指定最大条目数和 TTL。
    pub fn with_capacity(max_entries: u64, ttl: Duration) -> Self {
        Self { cache: RequestCache::new(max_entries, ttl) }
    }
}

#[async_trait]
impl Middleware for CacheMiddleware {
    fn priority(&self) -> u32 { 3 }

    async fn process_request(&self, req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        // 键含 method，避免 POST/GET 同 URL 串味（与 engine.rs 保持一致）
        let method_str = match req.method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        };
        if let Some(entry) = self.cache.get(method_str, &req.url).await {
            let resp = SpiderResponse {
                url: req.url.clone(),
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
                request: req.clone(),
                tracker: None,
                from_cache: true,
            };
            return MwAction::Respond(resp);
        }
        MwAction::Continue
    }

    async fn process_response(&self, resp: &mut SpiderResponse, _ctx: &CrawlContext) -> MwAction {
        if resp.status >= 200 && resp.status < 400 && !resp.from_cache {
            // 写入时用请求方法（resp.request.method）作为键的一部分
            let method_str = match resp.request.method {
                Method::Get => "GET",
                Method::Post => "POST",
                Method::Put => "PUT",
                Method::Delete => "DELETE",
            };
            self.cache.put(method_str, &resp.url, CachedEntry {
                status: resp.status,
                headers: resp.headers.clone(),
                body: resp.body.clone(),
            }).await;
        }
        MwAction::Continue
    }
}

/// Robots.txt 检查中间件：请求前检查目标 URL 是否被 robots.txt 禁止。
pub struct RobotsMiddleware {
    robots_cache: Arc<Mutex<RobotsCache>>,
    client: Arc<Client>,
}

impl RobotsMiddleware {
    pub fn new(robots_cache: Arc<Mutex<RobotsCache>>, client: Arc<Client>) -> Self {
        Self { robots_cache, client }
    }
}

#[async_trait]
impl Middleware for RobotsMiddleware {
    fn priority(&self) -> u32 { 8 }

    async fn process_request(&self, req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        let allowed = {
            let mut rc = self.robots_cache.lock().await;
            rc.is_allowed(&self.client, &req.url).await
        };
        if allowed { MwAction::Continue } else { MwAction::Skip }
    }
}

/// 下载延迟中间件：每个请求发出前等待固定时间，避免过快访问。
pub struct DelayMiddleware {
    delay: Duration,
}

impl DelayMiddleware {
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }

    /// 便捷构造：毫秒数。
    pub fn from_millis(ms: u64) -> Self {
        Self { delay: Duration::from_millis(ms) }
    }
}

#[async_trait]
impl Middleware for DelayMiddleware {
    fn priority(&self) -> u32 { 15 }

    async fn process_request(&self, _req: &mut SpiderRequest, _ctx: &CrawlContext) -> MwAction {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        MwAction::Continue
    }
}

// === 模式升级类 ===

/// Stealth 升级中间件：HTTP 被拦截时自动升级为 Stealth 浏览器模式重取。
pub struct StealthUpgradeMiddleware {
    rule_engine: Arc<Mutex<ModeRuleEngine>>,
}

impl StealthUpgradeMiddleware {
    pub fn new(rule_engine: Arc<Mutex<ModeRuleEngine>>) -> Self {
        Self { rule_engine }
    }
}

#[async_trait]
impl Middleware for StealthUpgradeMiddleware {
    fn priority(&self) -> u32 { 45 }

    async fn process_response(&self, resp: &mut SpiderResponse, _ctx: &CrawlContext) -> MwAction {
        if resp.request.fetch_mode_override == Some(FetchMode::Stealth) {
            return MwAction::Continue;
        }
        if auto::is_blocked_response(resp.status, &resp.body, &resp.headers) {
            self.rule_engine.lock().await.learn(&resp.url, FetchMode::Stealth);
            tracing::info!("StealthUpgrade: '{}' 被拦截 (status={})，升级 Stealth", resp.url, resp.status);
            let mut new_req = resp.request.clone();
            new_req.fetch_mode_override = Some(FetchMode::Stealth);
            return MwAction::Refetch(new_req);
        }
        MwAction::Continue
    }
}

// === 重试类 ===

/// 阻塞重试中间件：检测 403/429/503 等阻塞状态码，通过 Refetch 自动重试。
pub struct BlockedRetryMiddleware {
    max_retries: u32,
    retry_delay: Duration,
}

impl BlockedRetryMiddleware {
    pub fn new(max_retries: u32, retry_delay: Duration) -> Self {
        Self { max_retries, retry_delay }
    }
}

impl Default for BlockedRetryMiddleware {
    fn default() -> Self {
        Self { max_retries: 3, retry_delay: Duration::from_millis(500) }
    }
}

#[async_trait]
impl Middleware for BlockedRetryMiddleware {
    fn priority(&self) -> u32 { 80 }

    async fn process_response(&self, resp: &mut SpiderResponse, _ctx: &CrawlContext) -> MwAction {
        use crate::crawl::BLOCKED_STATUS_CODES;
        if BLOCKED_STATUS_CODES.contains(&resp.status) {
            let count = resp.request.meta.get("_retry").and_then(|v| v.as_u64()).unwrap_or(0);
            if count < self.max_retries as u64 {
                if !self.retry_delay.is_zero() {
                    tokio::time::sleep(self.retry_delay).await;
                }
                let mut new_req = resp.request.clone();
                new_req.meta["_retry"] = serde_json::json!(count + 1);
                return MwAction::Refetch(new_req);
            }
        }
        MwAction::Continue
    }
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use serde_json::Value;
    use super::super::pipeline::FilterFieldsPipeline;

    fn make_req() -> SpiderRequest {
        SpiderRequest {
            url: "http://example.com".into(),
            method: crate::crawl::Method::Get,
            headers: HashMap::new(),
            body: None,
            meta: Value::Null,
            callback: None,
            priority: 0,
            depth: 0,
            proxy: None,
            fetch_mode_override: None,
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
        assert_eq!(action, MwAction::Modified);
        assert!(req.headers.contains_key("User-Agent"));
    }

    #[tokio::test]
    async fn test_headers_middleware() {
        let mw = HeadersMiddleware::new(vec![("X-Custom".into(), "value1".into())]);
        let ctx = make_ctx();
        let mut req = make_req();
        let action = mw.process_request(&mut req, &ctx).await;
        assert_eq!(action, MwAction::Modified);
        assert_eq!(req.headers.get("X-Custom").unwrap(), "value1");
    }

    #[tokio::test]
    async fn test_retry_middleware() {
        let mw = RetryMiddleware::new(3, Duration::ZERO);
        let ctx = make_ctx();
        let req = make_req();
        let action = mw.process_error(&req, "timeout", &ctx).await;
        assert_eq!(action, ErrorAction::Retry);

        let mut req_max = make_req();
        req_max.meta = serde_json::json!({"_retry": 3});
        let action = mw.process_error(&req_max, "timeout", &ctx).await;
        assert_eq!(action, ErrorAction::Propagate);
    }

    #[tokio::test]
    async fn test_domain_filter_middleware() {
        let mw = DomainFilterMiddleware::new(["example.com", "api.example.com"]);
        let ctx = make_ctx();
        let mut req = make_req();
        req.url = "https://example.com/page".into();
        assert_eq!(mw.process_request(&mut req, &ctx).await, MwAction::Continue);

        let mut req2 = make_req();
        req2.url = "https://evil.com/page".into();
        assert_eq!(mw.process_request(&mut req2, &ctx).await, MwAction::Skip);
    }

    #[tokio::test]
    async fn test_depth_limit_middleware() {
        let mw = DepthLimitMiddleware::new(3);
        let ctx = make_ctx();
        let mut req = make_req();
        req.depth = 2;
        assert_eq!(mw.process_request(&mut req, &ctx).await, MwAction::Continue);
        let mut req2 = make_req();
        req2.depth = 4;
        assert_eq!(mw.process_request(&mut req2, &ctx).await, MwAction::Skip);
    }

    #[tokio::test]
    async fn test_cache_middleware() {
        let mw = CacheMiddleware::with_capacity(100, Duration::from_secs(60));
        let ctx = make_ctx();
        let mut req = make_req();
        assert_eq!(mw.process_request(&mut req, &ctx).await, MwAction::Continue);

        let mut resp = SpiderResponse {
            url: "http://example.com".into(),
            status: 200,
            headers: HashMap::new(),
            body: b"hello".to_vec(),
            request: req.clone(),
            tracker: None,
            from_cache: false,
        };
        mw.process_response(&mut resp, &ctx).await;

        let mut req2 = make_req();
        match mw.process_request(&mut req2, &ctx).await {
            MwAction::Respond(cached) => {
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
        assert_eq!(action, MwAction::Continue);
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
        let domain = DomainFilterMiddleware::new(["a.com"]);
        let cache = CacheMiddleware::with_capacity(10, Duration::from_secs(1));
        let depth = DepthLimitMiddleware::new(5);
        let headers = HeadersMiddleware::new(vec![]);
        let delay = DelayMiddleware::from_millis(0);
        let ua = UaRotationMiddleware::desktop();

        assert_eq!(domain.priority(), 0);
        assert_eq!(cache.priority(), 3);
        assert_eq!(depth.priority(), 5);
        assert_eq!(headers.priority(), 10);
        assert_eq!(delay.priority(), 15);
        assert_eq!(ua.priority(), 20);
    }
}
