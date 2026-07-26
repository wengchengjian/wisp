//! 内建中间件实现。

use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::{CrawlContext, ErrorAction, Middleware, MwAction};
use crate::crawl::auto::{self, ModeRuleEngine};
use crate::crawl::runtime::robots::RobotsCache;
use crate::crawl::{Request, Response};
use crate::fetcher::FetchMode;
use crate::http::Client;
use crate::storage::{CachedResponse, Store};

// === 请求修改类 ===

/// UA 轮换中间件：每次请求随机选择一个 User-Agent。
pub struct UaRotationMiddleware {
    agents: Vec<&'static str>,
    index: std::sync::atomic::AtomicUsize,
}

impl UaRotationMiddleware {
    /// 使用桌面 UA 列表创建（Chrome/Edge 136，匹配默认 TLS 指纹）。
    #[must_use]
    pub fn desktop() -> Self {
        Self {
            agents: vec![
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0",
            ],
            index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// 使用自定义 UA 列表创建。
    #[must_use]
    pub fn with_agents(agents: Vec<&'static str>) -> Self {
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
            .insert("User-Agent".to_string(), self.agents[idx].to_string());
        MwAction::Modified
    }
}

/// 重试中间件：决定网络错误是否值得重试。
///
/// 职责单一（修复 ND-002-CORR）：
/// - **只决定**：这个错误是否值得重试（业务决策）
/// - **不维护**：重试计数和上限由 engine 在 `fetch_dispatch` 内统一管理
///
/// engine 读取 `EngineConfig.max_retries` 作为上限，维护 `req.retry_count` 计数。
/// 中间件返回 `ErrorAction::Retry` 或 `Propagate`，退避策略（指数退避 + 抖动）
/// 由 engine 统一负责（避免与中间件 sleep 重复退避）。
pub struct RetryMiddleware {
    max_retries: u32,
}

impl RetryMiddleware {
    /// 创建重试中间件。
    ///
    /// - `max_retries`：网络错误最大重试次数（与 `EngineConfig.max_retries` 一致，
    ///   作为中间件层的双重保险，避免单点逻辑漂移）。
    ///   退避由 engine 统一负责（指数退避 + 抖动），中间件不做 sleep。
    #[must_use]
    pub fn new(max_retries: u32) -> Self {
        Self { max_retries }
    }
}

#[async_trait]
impl Middleware for RetryMiddleware {
    fn priority(&self) -> u32 {
        90
    }

    async fn process_error(&self, req: &Request, _err: &str, _ctx: &CrawlContext) -> ErrorAction {
        // fetch_page 返回 Err 都是网络层错误（DNS/连接/TLS/超时等），
        // HTTP 业务错误（4xx/5xx）会返回 Ok(resp)，由 BlockedRetryMiddleware 通过 Refetch 处理。
        // OPTIMIZE: 退避由 engine 统一负责（指数退避 + 抖动），中间件仅决定是否重试。
        if req.retry_count < self.max_retries {
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
    /// 创建代理注入中间件。
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
            req.proxy = Some(proxy.to_string());
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
    /// 创建请求头注入中间件。
    #[must_use]
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
    /// 创建 Cookie 挑战中间件。
    #[must_use]
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
            format!("{existing}; {cookie_pair}")
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
            allowed: allowed.into_iter().map(std::convert::Into::into).collect(),
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
    /// 创建深度限制中间件。
    #[must_use]
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
/// 响应返回后自动写入缓存。TTL 由 `default_ttl` 决定（写入 `CachedResponse.ttl`）。
pub struct CacheMiddleware {
    store: Arc<dyn Store>,
    default_ttl: Option<Duration>,
}

impl CacheMiddleware {
    /// 构造缓存中间件。`default_ttl` 为 `None` 时缓存永不过期。
    pub fn new(store: Arc<dyn Store>, default_ttl: Option<Duration>) -> Self {
        Self { store, default_ttl }
    }
}

#[async_trait]
impl Middleware for CacheMiddleware {
    fn priority(&self) -> u32 {
        3
    }

    async fn process_request(&self, req: &mut Request, _ctx: &CrawlContext) -> MwAction {
        let method_str = req.method.as_str();
        match crate::storage::load_response(&*self.store, method_str, &req.url).await {
            Ok(Some(cached)) => {
                let resp = Response::from_parts(
                    cached.status,
                    req.url.clone(),
                    cached.headers,
                    cached.body,
                    None,
                    Vec::new(),
                    req.clone(),
                    cached.content_type,
                    true,
                );
                return MwAction::Respond(resp);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("缓存读取失败: {}", e),
        }
        MwAction::Continue
    }

    async fn process_response(&self, resp: &mut Response, _ctx: &CrawlContext) -> MwAction {
        if resp.status >= 200 && resp.status < 400 && !resp.from_cache {
            let method_str = resp.request.method.as_str();
            let cached = CachedResponse {
                status: resp.status,
                headers: resp.headers.clone(),
                body: resp.body.clone(),
                content_type: resp.content_type.clone(),
                cached_at: chrono::Utc::now().timestamp(),
                ttl: self.default_ttl,
            };
            if let Err(e) =
                crate::storage::save_response(&*self.store, method_str, &resp.url, &cached).await
            {
                tracing::warn!("响应缓存写入失败: {}", e);
            }
        }
        MwAction::Continue
    }
}

/// Robots.txt 检查中间件：请求前检查目标 URL 是否被 robots.txt 禁止。
///
/// `RobotsCache` 内部用 `DashMap` 实现无锁读 + fetch 时不持锁，
/// 因此 `RobotsMiddleware` 无需额外 `Mutex` 包裹，多个并发请求可并行检查。
pub struct RobotsMiddleware {
    robots_cache: Arc<RobotsCache>,
    client: Arc<Client>,
}

impl RobotsMiddleware {
    /// 创建 Robots.txt 检查中间件。
    #[must_use]
    pub fn new(robots_cache: Arc<RobotsCache>, client: Arc<Client>) -> Self {
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
        let allowed = self.robots_cache.is_allowed(&self.client, &req.url).await;
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
    /// 创建下载延迟中间件。
    #[must_use]
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }

    /// 便捷构造：毫秒数。
    #[must_use]
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
    /// 创建 Stealth 升级中间件。
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
    /// 创建 Dynamic 升级中间件。
    #[must_use]
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
    ///
    /// 大 body (>1MB) 用 `spawn_blocking` 移到 blocking pool，避免阻塞 tokio worker；
    /// 小 body 直接同步执行，避免 spawn 开销。
    pub async fn score_body(&self, body: &[u8]) -> u8 {
        const LARGE_BODY_THRESHOLD: usize = 1 << 20; // 1MB

        if body.len() > LARGE_BODY_THRESHOLD {
            // 大 body 移到 blocking pool
            let body_vec = body.to_vec();
            let spa = self.spa_matcher.clone();
            let dom = self.dom_matcher.clone();
            let script = self.script_matcher.clone();
            tokio::task::spawn_blocking(move || {
                Self::score_body_sync(&body_vec, &spa, &dom, &script)
            })
            .await
            .expect("score_body spawn_blocking join failed")
        } else {
            // 小 body 直接同步执行
            Self::score_body_sync(
                body,
                &self.spa_matcher,
                &self.dom_matcher,
                &self.script_matcher,
            )
        }
    }

    /// score_body 的同步实现（纯 CPU 计算，无 await）。
    fn score_body_sync(
        body: &[u8],
        spa_matcher: &aho_corasick::AhoCorasick,
        dom_matcher: &aho_corasick::AhoCorasick,
        script_matcher: &aho_corasick::AhoCorasick,
    ) -> u8 {
        // 强信号：SPA 框架标识 → 直接满分
        if spa_matcher.find(body).is_some() {
            return 10;
        }
        // 中信号：DOM 修改方法 → 7 分
        if dom_matcher.find(body).is_some() {
            return 7;
        }
        // 弱信号：`<script` 标签密度 >= 6 → 7 分（短路扫描，达到阈值即停）
        if script_matcher
            .find_iter(body)
            .take(SCRIPT_DENSITY_THRESHOLD)
            .count()
            >= SCRIPT_DENSITY_THRESHOLD
        {
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
        if self.score_body(&resp.body).await >= 7 {
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
///
/// 职责单一（修复 ND-032-CORR：原 `_retry` 计数与 `RetryMiddleware` 共享 meta 字段冲突）：
/// - **只决定**：响应是否被阻塞、是否值得重试
/// - **不维护**：Refetch 计数由 engine 在 `process_response` 内通过 `refetch_depth` 管理，
///   上限 `EngineConfig.max_refetch_rounds`
///
/// 原 `meta["_retry"]` 计数已移除，避免与 `RetryMiddleware` 的网络错误重试计数冲突。
/// 现在两套重试完全独立：
/// - 网络错误重试：`req.retry_count`（engine 维护，上限 `max_retries`）
/// - 阻塞重试：`refetch_depth`（engine 维护，上限 `max_refetch_rounds`）
pub struct BlockedRetryMiddleware {
    retry_delay: Duration,
}

impl BlockedRetryMiddleware {
    /// 创建阻塞重试中间件。
    #[must_use]
    pub fn new(retry_delay: Duration) -> Self {
        Self { retry_delay }
    }
}

impl Default for BlockedRetryMiddleware {
    fn default() -> Self {
        Self {
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
        // Stealth 模式下不再 Refetch：浏览器已是最强抓取模式，重复 Refetch 只会
        // 再次触发 CF 挑战流程，无法突破拦截。状态码 403/503 通常是挑战前的响应，
        // 挑战已由 ChallengeSolver 解决（fetch_browser 内会修正最终状态码）。
        // 与 StealthUpgradeMiddleware 的防御保持一致。
        if resp.request.fetch_mode_override == Some(FetchMode::Stealth) {
            return MwAction::Continue;
        }
        if BLOCKED_STATUS_CODES.contains(&resp.status) {
            // 不再维护 _retry 计数，依赖 engine 的 refetch_depth 上限
            if !self.retry_delay.is_zero() {
                tokio::time::sleep(self.retry_delay).await;
            }
            let new_req = resp.request.clone();
            return MwAction::Refetch(new_req);
        }
        MwAction::Continue
    }
}

// === 默认中间件注入 ===

/// 默认中间件注入配置（由 Spider 配置 + Engine 资源组装）。
///
/// `default_middlewares` 据此构造中间件链；字段对应各默认中间件所需的输入。
pub struct DefaultMiddlewareConfig {
    /// 抓取模式（决定是否注入模式升级类）
    pub fetch_mode: FetchMode,
    /// 下载延迟（>0 时注入 DelayMiddleware）
    pub delay: Duration,
    /// 是否遵守 robots.txt（true 时注入 RobotsMiddleware）
    pub obey_robots: bool,
    /// 允许的域名集合（非空时注入 DomainFilterMiddleware）
    pub allowed_domains: HashSet<String>,
    /// 最大深度（注入 DepthLimitMiddleware；MAX 时等价无限制）
    pub max_depth: u32,
    /// 响应缓存存储（Some 时注入 CacheMiddleware，永不过期）
    pub cache_store: Option<Arc<dyn Store>>,
    /// HTTP 客户端（RobotsMiddleware 拉取 robots.txt 用）
    pub http_client: Arc<Client>,
    /// robots 缓存（跨请求共享 robots 规则，内部 DashMap 无锁读）
    pub robots_cache: Arc<RobotsCache>,
    /// Auto 模式规则引擎（StealthUpgradeMiddleware 学习模式用）
    pub rule_engine: Arc<Mutex<ModeRuleEngine>>,
    /// 网络错误最大重试次数（注入 RetryMiddleware；与 EngineConfig.max_retries 一致）
    pub max_retries: u32,
}

/// 按 FetchMode 和 Spider 配置注入默认行为中间件链。
///
/// 中间件分 4 类（按 priority 升序，由 `MiddlewareChain::sort` 统一排序）：
/// 1. **过滤类**（0-8）：DomainFilter / Cache / DepthLimit / Robots — 按配置启用
/// 2. **请求修改类**（10-30）：Delay / UaRotation — delay>0 / 总是
/// 3. **模式升级类**（40-45）：DynamicUpgrade / StealthUpgrade — **仅 Auto 模式**
/// 4. **重试/挑战类**（50-90）：CookieChallenge / BlockedRetry / Retry — 总是
///
/// Http/Dynamic/Stealth 模式不注入升级类（用户已明确选择模式）；
/// Auto 模式注入完整链，由响应中间件按需 `Refetch` 升级。
///
/// 用户通过 `SpiderBuilder::middleware` 添加的中间件视为额外自定义，
/// 与默认链合并后统一排序。请勿重复添加同类 builtin 中间件。
#[must_use]
pub fn default_middlewares(cfg: DefaultMiddlewareConfig) -> Vec<Arc<dyn Middleware>> {
    let mut mws: Vec<Arc<dyn Middleware>> = Vec::new();

    // 1. 过滤类
    if !cfg.allowed_domains.is_empty() {
        mws.push(Arc::new(DomainFilterMiddleware::new(cfg.allowed_domains)));
    }
    if let Some(store) = cfg.cache_store.clone() {
        mws.push(Arc::new(CacheMiddleware::new(store, None)));
    }
    mws.push(Arc::new(DepthLimitMiddleware::new(cfg.max_depth)));
    if cfg.obey_robots {
        mws.push(Arc::new(RobotsMiddleware::new(
            cfg.robots_cache,
            cfg.http_client,
        )));
    }

    // 2. 请求修改类
    if !cfg.delay.is_zero() {
        mws.push(Arc::new(DelayMiddleware::new(cfg.delay)));
    }
    mws.push(Arc::new(UaRotationMiddleware::desktop()));

    // 3. 模式升级类（仅 Auto）
    if cfg.fetch_mode == FetchMode::Auto {
        mws.push(Arc::new(DynamicUpgradeMiddleware::new()));
        mws.push(Arc::new(StealthUpgradeMiddleware::new(cfg.rule_engine)));
    }

    // 4. 重试/挑战类
    mws.push(Arc::new(CookieChallengeMiddleware::default()));
    mws.push(Arc::new(BlockedRetryMiddleware::default()));
    mws.push(Arc::new(RetryMiddleware::new(cfg.max_retries)));

    mws
}

// === 测试 ===

#[cfg(test)]
mod tests {
    use super::super::pipeline::FilterFieldsPipeline;
    use super::super::ItemPipeline;
    use super::*;
    use crate::crawl::auto::ModeRuleEngine;
    use crate::crawl::runtime::robots::RobotsCache;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

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
    async fn test_retry_middleware_always_retries_fetch_errors() {
        // RetryMiddleware 在 retry_count < max_retries 时返回 Retry
        // fetch_page 返回 Err 都是网络层错误，均可重试（直到耗尽 max_retries）
        let mw = RetryMiddleware::new(3);
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
            let action = mw.process_error(&req, err, &ctx).await;
            assert_eq!(action, ErrorAction::Retry, "网络错误 '{err}' 应可重试");
        }
    }

    /// retry_count 已达 max_retries 时应返回 Propagate（不再重试）。
    #[tokio::test]
    async fn test_retry_middleware_propagates_when_retries_exhausted() {
        let mw = RetryMiddleware::new(2);
        let ctx = make_ctx();
        let mut req = make_req();
        req.retry_count = 2;
        let action = mw.process_error(&req, "connection refused", &ctx).await;
        assert_eq!(
            action,
            ErrorAction::Propagate,
            "retry_count == max_retries 应返回 Propagate"
        );
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
        let mw = CacheMiddleware::new(
            Arc::new(crate::storage::MemoryStore::default()),
            Some(Duration::from_mins(1)),
        );
        let ctx = make_ctx();
        let mut req = make_req();
        assert_eq!(mw.process_request(&mut req, &ctx).await, MwAction::Continue);

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
            MwAction::Respond(cached) => {
                assert_eq!(cached.status, 200);
                assert!(cached.from_cache);
            }
            other => panic!("expected Respond, got {other:?}"),
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
        let cache = CacheMiddleware::new(
            Arc::new(crate::storage::MemoryStore::default()),
            Some(Duration::from_secs(1)),
        );
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
            MwAction::Refetch(req) => {
                assert_eq!(req.fetch_mode_override, Some(FetchMode::Dynamic));
            }
            other => panic!("expected Refetch, got {other:?}"),
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
            other => panic!("expected Refetch, got {other:?}"),
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
            other => panic!("expected Refetch, got {other:?}"),
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

    /// M8：验证大 body（>1MB）score_body 不阻塞 tokio runtime。
    ///
    /// OPTIMIZE: 旧实现直接在 async worker 上执行 aho-corasick + HTML 解析，
    /// 大 body 会阻塞 worker。改用 spawn_blocking 后，大 body 移到 blocking pool。
    #[tokio::test]
    async fn test_score_body_large_does_not_block_runtime() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{Duration, Instant};

        // 构造一个 2MB 的 HTML body（> 1MB 阈值）
        let large_body: Vec<u8> = b"<html><body>".repeat(200_000); // ~2.4MB

        let middleware = DynamicUpgradeMiddleware::new();
        let counter = Arc::new(AtomicU32::new(0));
        let c = Arc::clone(&counter);

        // 后台 task：每 1ms 自增 counter
        let task = tokio::spawn(async move {
            for _ in 0..100 {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        // 同时执行 score_body（应 spawn_blocking，不阻塞 runtime）
        let start = Instant::now();
        let _score = middleware.score_body(&large_body).await;
        let elapsed = start.elapsed();

        task.await.unwrap();

        let counter_val = counter.load(Ordering::SeqCst);
        // 后台 task 应在 score_body 期间继续推进（spawn_blocking 不占用 worker）
        assert!(
            counter_val > 50,
            "后台 task 应在 score_body 期间继续，实际 counter={counter_val}"
        );
        // score_body 应 < 2s（spawn_blocking 不阻塞）
        assert!(
            elapsed < Duration::from_secs(2),
            "score_body 应 < 2s，实际 {elapsed:?}"
        );
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
            MwAction::Refetch(_) => {}
            other => panic!("expected Refetch, got {other:?}"),
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
        assert_eq!(action, MwAction::Continue);
    }

    #[tokio::test]
    async fn blocked_retry_skips_for_200() {
        // 200 正常响应不触发 Refetch
        let mw = BlockedRetryMiddleware::new(Duration::ZERO);
        let ctx = make_ctx();
        let mut resp = make_resp(200, b"<html>ok</html>");
        let action = mw.process_response(&mut resp, &ctx).await;
        assert_eq!(action, MwAction::Continue);
    }

    // === default_middlewares 分类逻辑测试 ===

    /// 构造测试用 HTTP Client（从 FetchClient 提取，复用 engine 测试模式）。
    fn make_http_client() -> Arc<Client> {
        crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
            .expect("build fetch client")
            .http_arc()
    }

    /// 验证默认中间件按 fetch_mode + 配置正确分类注入。
    #[test]
    fn default_middlewares_classifies_by_mode_and_config() {
        let http_client = make_http_client();
        let robots_cache = Arc::new(RobotsCache::new());
        let rule_engine = Arc::new(Mutex::new(ModeRuleEngine::new()));

        let priorities =
            |mws: &[Arc<dyn Middleware>]| mws.iter().map(|m| m.priority()).collect::<Vec<_>>();

        // Auto + 全配置：应注入完整链（含模式升级类 40/45）
        let auto_p = priorities(&default_middlewares(DefaultMiddlewareConfig {
            fetch_mode: FetchMode::Auto,
            delay: Duration::from_millis(100),
            obey_robots: true,
            allowed_domains: ["example.com".to_string()].into_iter().collect(),
            max_depth: 3,
            cache_store: None,
            http_client: http_client.clone(),
            robots_cache: robots_cache.clone(),
            rule_engine: rule_engine.clone(),
            max_retries: 3,
        }));
        assert!(auto_p.contains(&0), "allowed 非空应含 DomainFilter");
        assert!(auto_p.contains(&5), "应含 DepthLimit");
        assert!(auto_p.contains(&8), "obey_robots=true 应含 Robots");
        assert!(auto_p.contains(&15), "delay>0 应含 Delay");
        assert!(auto_p.contains(&20), "应含 UaRotation");
        assert!(auto_p.contains(&40), "Auto 应含 DynamicUpgrade");
        assert!(auto_p.contains(&45), "Auto 应含 StealthUpgrade");
        assert!(auto_p.contains(&50), "应含 CookieChallenge");
        assert!(auto_p.contains(&80), "应含 BlockedRetry");
        assert!(auto_p.contains(&90), "应含 Retry");

        // Http + 最小配置：不应含升级类和条件类
        let http_p = priorities(&default_middlewares(DefaultMiddlewareConfig {
            fetch_mode: FetchMode::Http,
            delay: Duration::ZERO,
            obey_robots: false,
            allowed_domains: HashSet::new(),
            max_depth: u32::MAX,
            cache_store: None,
            http_client: http_client.clone(),
            robots_cache: robots_cache.clone(),
            rule_engine: rule_engine.clone(),
            max_retries: 3,
        }));
        assert!(!http_p.contains(&0), "allowed 空不应含 DomainFilter");
        assert!(!http_p.contains(&8), "obey_robots=false 不应含 Robots");
        assert!(!http_p.contains(&15), "delay=0 不应含 Delay");
        assert!(!http_p.contains(&40), "Http 不应含 DynamicUpgrade");
        assert!(!http_p.contains(&45), "Http 不应含 StealthUpgrade");
        // 总是注入的仍存在
        assert!(http_p.contains(&5), "总是含 DepthLimit");
        assert!(http_p.contains(&20), "总是含 UaRotation");
        assert!(http_p.contains(&50), "总是含 CookieChallenge");
        assert!(http_p.contains(&80), "总是含 BlockedRetry");
        assert!(http_p.contains(&90), "总是含 Retry");

        // Dynamic/Stealth 模式同样不注入升级类
        for mode in [FetchMode::Dynamic, FetchMode::Stealth] {
            let p = priorities(&default_middlewares(DefaultMiddlewareConfig {
                fetch_mode: mode,
                delay: Duration::ZERO,
                obey_robots: false,
                allowed_domains: HashSet::new(),
                max_depth: u32::MAX,
                cache_store: None,
                http_client: http_client.clone(),
                robots_cache: robots_cache.clone(),
                rule_engine: rule_engine.clone(),
                max_retries: 3,
            }));
            assert!(!p.contains(&40), "{mode:?} 不应含 DynamicUpgrade");
            assert!(!p.contains(&45), "{mode:?} 不应含 StealthUpgrade");
        }
    }
}
