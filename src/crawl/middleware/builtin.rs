//! 内建中间件实现。

use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::{CrawlContext, ErrorAction, Middleware, MwAction};
use crate::crawl::auto::{self, ModeRuleEngine};
use crate::crawl::runtime::request_cache::{CachedEntry, RequestCache};
use crate::crawl::runtime::robots::RobotsCache;
use crate::crawl::{Request, Response};
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
        Self {
            agents,
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Middleware for UaRotationMiddleware {
    fn priority(&self) -> u32 {
        20
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
        if self.agents.is_empty() {
            return MwAction::Continue;
        }
        let idx = self
            .index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.agents.len();
        req.headers
            .insert("User-Agent".to_string(), self.agents[idx].clone());
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
        Self {
            max_retries,
            retry_delay,
        }
    }
}

#[async_trait]
impl Middleware for RetryMiddleware {
    fn priority(&self) -> u32 {
        90
    }

    async fn process_error(
        &self,
        req: &Request,
        _err: &str,
        _ctx: &CrawlContext,
    ) -> ErrorAction {
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
    fn priority(&self) -> u32 {
        30
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
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
    fn priority(&self) -> u32 {
        10
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
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
    fn priority(&self) -> u32 {
        50
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> MwAction {
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
        let existing = resp
            .request
            .headers
            .get("Cookie")
            .cloned()
            .unwrap_or_default();
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
        Self {
            allowed: allowed.into_iter().map(|s| s.into()).collect(),
        }
    }
}

#[async_trait]
impl Middleware for DomainFilterMiddleware {
    fn priority(&self) -> u32 {
        0
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
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
    fn priority(&self) -> u32 {
        5
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
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
        Self {
            cache: RequestCache::new(max_entries, ttl),
        }
    }
}

#[async_trait]
impl Middleware for CacheMiddleware {
    fn priority(&self) -> u32 {
        3
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
        // 键含 method，避免 POST/GET 同 URL 串味（与 engine.rs 保持一致）
        let method_str = req.method.as_str();
        if let Some(entry) = self.cache.get(method_str, &req.url).await {
            let resp = Response {
                url: req.url.clone(),
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
                title: None,
                cookies: Vec::new(),
                request: req.clone(),
                content_type: String::new(),
                from_cache: true,
            };
            return MwAction::Respond(resp);
        }
        MwAction::Continue
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> MwAction {
        if resp.status >= 200 && resp.status < 400 && !resp.from_cache {
            // 写入时用请求方法（resp.request.method）作为键的一部分
            let method_str = resp.request.method.as_str();
            self.cache
                .put(
                    method_str,
                    &resp.url,
                    CachedEntry {
                        status: resp.status,
                        headers: resp.headers.clone(),
                        body: resp.body.clone(),
                    },
                )
                .await;
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
        Self {
            robots_cache,
            client,
        }
    }
}

#[async_trait]
impl Middleware for RobotsMiddleware {
    fn priority(&self) -> u32 {
        8
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
        let allowed = {
            let mut rc = self.robots_cache.lock().await;
            rc.is_allowed(&self.client, &req.url).await
        };
        if allowed {
            MwAction::Continue
        } else {
            MwAction::Skip
        }
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
        Self {
            delay: Duration::from_millis(ms),
        }
    }
}

#[async_trait]
impl Middleware for DelayMiddleware {
    fn priority(&self) -> u32 {
        15
    }

    async fn process_request(&self, _req: &mut Request, _ctx: &CrawlContext) -> MwAction {
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
    fn priority(&self) -> u32 {
        45
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> MwAction {
        if resp.request.fetch_mode_override == Some(FetchMode::Stealth) {
            return MwAction::Continue;
        }
        if auto::is_blocked_response(resp.status, &resp.body, &resp.headers) {
            self.rule_engine
                .lock()
                .await
                .learn(&resp.url, FetchMode::Stealth);
            tracing::info!(
                "StealthUpgrade: '{}' 被拦截 (status={})，升级 Stealth",
                resp.url,
                resp.status
            );
            let mut new_req = resp.request.clone();
            new_req.fetch_mode_override = Some(FetchMode::Stealth);
            return MwAction::Refetch(new_req);
        }
        MwAction::Continue
    }
}

// === Dynamic 升级类 ===

/// SPA 框架标识：命中任一即为强信号（10 分），立即升级。
const SPA_FRAMEWORK_MARKERS: &[&str] = &[
    "__NUXT_DATA__",
    "__NEXT_DATA__",
    "react-app.embeddedData",
    "data-reactroot",
    "ng-version",
    "data-v-app",
    "gatsby-chunk-mapping",
    "/_nuxt/",
    "/_next/static/",
];

/// DOM 修改方法：命中任一即为中信号（7 分）。
const DOM_MUTATION_METHODS: &[&str] = &[
    ".createElement(",
    ".innerHTML",
    ".outerHTML",
    "history.pushState",
    "history.replaceState",
    "fetch(",
    "new XMLHttpRequest",
];

/// 弱信号阈值：外部脚本密度 >= 此值时触发（7 分）。
/// 借鉴 spider 框架的 `script_src_count >= 4`，但 wisp 统计所有 `<script` 标签
/// （无法流式提取 src 属性），因此阈值调高为 6。
const SCRIPT_DENSITY_THRESHOLD: usize = 6;

/// Dynamic 升级中间件：检测页面可能需要 JS 渲染时升级到 Dynamic 模式。
///
/// 评分信号借鉴 spider 框架的 smart 模式：
/// - 强信号（10 分）：SPA 框架标识（`__NUXT_DATA__`、`__NEXT_DATA__` 等）
/// - 中信号（7 分）：DOM 修改方法（`.createElement(`、`.innerHTML`、`fetch(` 等）
/// - 弱信号（7 分）：`<script` 标签密度 >= 6
///
/// 评分 >= 7 时触发 `Refetch` + `fetch_mode_override = Dynamic`。
pub struct DynamicUpgradeMiddleware {
    spa_matcher: aho_corasick::AhoCorasick,
    dom_matcher: aho_corasick::AhoCorasick,
    script_matcher: aho_corasick::AhoCorasick,
}

impl Default for DynamicUpgradeMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicUpgradeMiddleware {
    pub fn new() -> Self {
        Self {
            spa_matcher: aho_corasick::AhoCorasick::new(SPA_FRAMEWORK_MARKERS)
                .expect("SPA markers should be valid"),
            dom_matcher: aho_corasick::AhoCorasick::new(DOM_MUTATION_METHODS)
                .expect("DOM mutation methods should be valid"),
            script_matcher: aho_corasick::AhoCorasick::new(["<script"])
                .expect("script pattern should be valid"),
        }
    }

    /// 评估响应 body 的 JS 渲染需求分数。
    fn score_body(&self, body: &[u8]) -> u8 {
        // 强信号：SPA 框架标识 → 直接满分
        if self.spa_matcher.find(body).is_some() {
            return 10;
        }
        // 中信号：DOM 修改方法 → 7 分
        if self.dom_matcher.find(body).is_some() {
            return 7;
        }
        // 弱信号：`<script` 标签密度 >= 6 → 7 分
        if self.script_matcher.find_iter(body).count() >= SCRIPT_DENSITY_THRESHOLD {
            return 7;
        }
        0
    }
}

#[async_trait]
impl Middleware for DynamicUpgradeMiddleware {
    fn priority(&self) -> u32 {
        40
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> MwAction {
        // 已有 override 不重复升级
        if resp.request.fetch_mode_override.is_some() {
            return MwAction::Continue;
        }
        // 仅对 200 响应检测
        if resp.status != 200 {
            return MwAction::Continue;
        }
        if self.score_body(&resp.body) >= 7 {
            let mut new_req = resp.request.clone();
            new_req.fetch_mode_override = Some(FetchMode::Dynamic);
            tracing::info!(
                "DynamicUpgrade: '{}' 检测到 SPA/DOM 动态特征，升级 Dynamic",
                resp.url
            );
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
        Self {
            max_retries,
            retry_delay,
        }
    }
}

impl Default for BlockedRetryMiddleware {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay: Duration::from_millis(500),
        }
    }
}

#[async_trait]
impl Middleware for BlockedRetryMiddleware {
    fn priority(&self) -> u32 {
        80
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> MwAction {
        use crate::crawl::BLOCKED_STATUS_CODES;
        if BLOCKED_STATUS_CODES.contains(&resp.status) {
            let count = resp
                .request
                .meta
                .get("_retry")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
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
    use super::super::pipeline::FilterFieldsPipeline;
    use super::super::ItemPipeline;
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;

    fn make_req() -> Request {
        Request {
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

        let mut resp = Response {
            url: "http://example.com".into(),
            status: 200,
            headers: HashMap::new(),
            body: b"hello".to_vec(),
            request: req.clone(),
            title: None,
            cookies: Vec::new(),
            content_type: String::new(),
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

    // === DynamicUpgradeMiddleware 测试 ===

    fn make_resp(status: u16, body: &[u8]) -> Response {
        Response {
            url: "http://example.com".into(),
            status,
            headers: HashMap::new(),
            body: body.to_vec(),
            request: make_req(),
            title: None,
            cookies: Vec::new(),
            content_type: String::new(),
            from_cache: false,
        }
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
            MwAction::Refetch(req) => {
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
            MwAction::Refetch(req) => {
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
        assert_eq!(action, MwAction::Continue);
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
        assert_eq!(action, MwAction::Continue);
    }

    #[tokio::test]
    async fn dynamic_upgrade_skips_non_200() {
        let mw = DynamicUpgradeMiddleware::new();
        let ctx = make_ctx();
        let mut resp = make_resp(403, b"<script id=\"__NUXT_DATA__\">{}</script>");
        let action = mw.process_response(&mut resp, &ctx).await;
        assert_eq!(action, MwAction::Continue);
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
            MwAction::Refetch(req) => {
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
        assert_eq!(action, MwAction::Continue);
    }
}
