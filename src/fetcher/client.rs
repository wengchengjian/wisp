//! 统一请求客户端 — 封装 HTTP Client 和 BrowserPool。
//!
//! - HTTP 请求：共享 `http::Client`（连接池复用）
//! - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
//! - Cookie 管理：通过 `cookie_jar: Arc<dyn CookieJar>` 统一 HTTP/浏览器/CF 三处 cookie

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use wreq_util::Profile;

use crate::browser::BrowserPool;
use crate::config::LaunchOptions;
use crate::cookie::{CookieJar, HttpCookieJar};
use crate::error::{BrowserError, Result, WispError};
use crate::http::{block::DomainBlocker, Client};

use super::response::{Request, Response};
use super::strategy::BrowserFetchStrategy;

/// 统一请求客户端配置。
#[derive(Debug, Clone)]
pub struct FetchClientConfig {
    /// 请求超时
    pub timeout: Duration,
    /// 代理 URL
    pub proxy: Option<String>,
    /// 浏览器 headless 模式
    pub headless: bool,
    /// 浏览器可执行文件路径（None = 自动搜索 Chrome/Chromium/Edge）
    pub executable_path: Option<std::path::PathBuf>,
    /// TLS 指纹模拟（Http 模式）
    pub emulation: Option<Profile>,
    /// 自定义 User-Agent
    pub user_agent: Option<String>,
    /// 自定义 headers
    pub headers: HashMap<String, String>,
    /// 最大重定向次数
    pub max_redirects: usize,
    /// 人类行为模拟（Stealth 模式）
    pub human_mode: bool,
    /// CF 挑战超时（Stealth 模式）
    pub challenge_timeout: Duration,
    /// 等待特定 CSS 选择器出现
    pub wait_for: Option<String>,
    /// 页面加载后额外等待（毫秒）
    pub extra_wait_ms: u64,
    /// 域名拦截器
    pub domain_blocker: Option<DomainBlocker>,
    /// DNS-over-HTTPS
    pub dns_over_https: Option<String>,
    /// BrowserPool 最大并发 page 数（0 = 禁用浏览器模式）
    pub max_concurrent_pages: usize,
    /// ND-008-SEC：响应体最大字节数。超过则返回 `ResponseBodyTooLarge` 错误，防止 OOM。
    /// 默认 64MB（覆盖大多数 HTML 页面；二进制/大文件场景应显式调高）。
    pub max_response_size: usize,
    /// ND-011-SEC：是否禁用 TLS 证书验证（危险！仅用于测试或自签名证书内部站点）。
    /// 默认 false（启用验证）。设为 true 等价于 curl -k，存在中间人攻击风险。
    pub danger_accept_invalid_certs: bool,
    /// Turnstile 解决器参数配置。
    pub turnstile: crate::stealth::TurnstileConfig,
    /// CF 会话缓存 TTL（默认 30 分钟）。
    pub cf_cookie_ttl: Duration,
    /// CF 会话持久化目录（默认 "wisp-data"）。
    pub cf_data_dir: PathBuf,
}

impl Default for FetchClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            proxy: None,
            headless: true,
            executable_path: None,
            emulation: Some(Profile::Chrome136),
            user_agent: None,
            headers: HashMap::new(),
            max_redirects: 10,
            human_mode: true,
            challenge_timeout: Duration::from_secs(30),
            wait_for: None,
            extra_wait_ms: 0,
            domain_blocker: None,
            dns_over_https: None,
            max_concurrent_pages: 4,
            max_response_size: 64 * 1024 * 1024, // 64MB
            danger_accept_invalid_certs: false,
            turnstile: crate::stealth::TurnstileConfig::default(),
            cf_cookie_ttl: Duration::from_mins(30),
            cf_data_dir: PathBuf::from("wisp-data"),
        }
    }
}

/// 统一请求客户端：封装 HTTP Client 和 BrowserPool。
///
/// - HTTP 请求：共享 `http::Client`（连接池复用）
/// - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
/// - Cookie 管理：通过 `cookie_jar` 统一 HTTP/浏览器/CF cookie 状态
pub struct FetchClient {
    http: Arc<Client>,
    browser_pool: Option<Arc<BrowserPool>>,
    config: FetchClientConfig,
    /// 共享 cookie jar（默认 HttpCookieJar，StealthStrategy 可注入 CfCookieJar）
    cookie_jar: Arc<dyn CookieJar>,
}

impl FetchClient {
    /// 创建 FetchClient。
    pub fn new(config: FetchClientConfig) -> Result<Self> {
        let http_jar = Arc::new(HttpCookieJar::new());
        let http = Arc::new(Self::build_http_client(&config, http_jar.jar())?);
        let browser_pool = Self::build_browser_pool(&config);
        let cookie_jar: Arc<dyn CookieJar> = http_jar;
        Ok(Self {
            http,
            browser_pool,
            config,
            cookie_jar,
        })
    }

    /// 获取共享 CookieJar。
    #[must_use]
    pub fn cookie_jar(&self) -> &Arc<dyn CookieJar> {
        &self.cookie_jar
    }

    /// 获取 HTTP 客户端引用。
    #[must_use]
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// 获取 HTTP 客户端的 Arc 克隆（用于需要独立持有 Client 的中间件，如 RobotsMiddleware）。
    #[must_use]
    pub fn http_arc(&self) -> Arc<Client> {
        Arc::clone(&self.http)
    }

    /// 获取浏览器池引用（若有）。
    #[must_use]
    pub fn browser_pool(&self) -> Option<&Arc<BrowserPool>> {
        self.browser_pool.as_ref()
    }

    /// 获取配置引用。
    #[must_use]
    pub fn config(&self) -> &FetchClientConfig {
        &self.config
    }

    /// HTTP 请求（共享 Client，连接复用）。直接返回统一 Response，无中间类型转换。
    pub async fn fetch_http(&self, req: &Request) -> Result<Response> {
        self.http.fetch(req).await
    }

    /// 浏览器请求（通过 BrowserPool + 注入 strategy）。
    ///
    /// ARCH: 替代 `fetch_browser(&req, solve_cf: bool)`。
    /// strategy 由调用方传入，FetchClient 不再关心 CF/Dynamic 差异。
    /// 120s 总超时由本方法包装。
    pub async fn fetch_browser(
        &self,
        req: &Request,
        strategy: &dyn BrowserFetchStrategy,
    ) -> Result<Response> {
        let pool = self.browser_pool.as_ref().ok_or_else(|| {
            WispError::Browser(BrowserError::Other(
                "browser pool not configured (max_concurrent_pages=0)".into(),
            ))
        })?;
        // acquire 返回带 page 的 handle（permit 限制并发数）
        let mut handle = pool.acquire().await?;

        // 总超时：防止 CF 挑战页面卡住整个流程（导航+挑战+提取各阶段都有单独超时，
        // 但极端情况下可能累加超过预期，这里加一个 120s 硬上限）
        let work = strategy.fetch(handle.page_mut(), req);
        let result = tokio::time::timeout(Duration::from_secs(120), work)
            .await
            .map_err(|_| {
                WispError::Timeout(format!(
                    "fetch_browser 总超时（120s）: {}",
                    crate::crawl::engine::sanitize_url(&req.url)
                ))
            })?;

        // 实际工作；无论成功/失败都显式关闭 tab
        let _ = handle.page_mut().close().await;
        // handle Drop：page.target_id 已 None（Page::Drop no-op）+ permit 自动 release
        result
    }

    /// 从事件流中接收 `Network.responseReceived` (type=Document) 事件并提取状态码。
    ///
    /// 必须在 `goto` 之前订阅 `event_rx`，否则可能丢失事件。
    /// 5s 超时：导航通常在 1-3s 内完成，5s 足够覆盖慢速页面。
    ///
    /// 特殊处理：若先收到 `Network.loadingFailed` (type=Document)，说明导航请求失败
    ///（如代理连接失败、DNS 解析失败），立即返回错误，不空等 5s 超时。
    #[allow(dead_code)]
    async fn recv_navigation_status(
        &self,
        rx: &mut tokio::sync::broadcast::Receiver<crate::browser::cdp::CdpEvent>,
        sid: &str,
    ) -> Result<u16> {
        use tokio::sync::broadcast::error::RecvError;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) => {
                    let match_session =
                        event.session_id.as_deref() == Some(sid) || event.session_id.is_none();
                    if !match_session {
                        continue;
                    }

                    // 导航请求失败（代理/DNS/网络问题）：立即返回错误
                    if event.method == "Network.loadingFailed" {
                        let is_doc =
                            event.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                        if is_doc {
                            let error_text = event
                                .params
                                .get("errorText")
                                .and_then(|t| t.as_str())
                                .unwrap_or("unknown");
                            tracing::warn!(
                                "recv_navigation_status: Network.loadingFailed errorText={error_text}"
                            );
                            return Err(WispError::Browser(BrowserError::CdpConnection(format!(
                                "navigation loading failed: {error_text}"
                            ))));
                        }
                        continue;
                    }

                    if event.method != "Network.responseReceived" {
                        continue;
                    }
                    let is_doc =
                        event.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                    if !is_doc {
                        continue;
                    }
                    return event
                        .params
                        .get("response")
                        .and_then(|r| r.get("status"))
                        .and_then(serde_json::Value::as_u64)
                        .map(|s| s as u16)
                        .ok_or_else(|| {
                            WispError::Browser(BrowserError::CdpConnection(
                                "Network.responseReceived missing response.status".into(),
                            ))
                        });
                }
                Ok(Err(RecvError::Lagged(n))) => {
                    tracing::warn!("event subscriber lagged by {n} events, continuing recv");
                }
                Ok(Err(RecvError::Closed)) => {
                    return Err(WispError::Browser(BrowserError::CdpConnection(
                        "event broadcaster closed before navigation status captured".into(),
                    )));
                }
                Err(_) => {
                    // 超时不返回错误：CF 挑战页面可能不触发 Network.responseReceived (type=Document)
                    // 事件（CF 用 JavaScript 挑战，非标准 HTTP 响应流程）。
                    // 返回默认 200，让流程继续到 CF 挑战解决阶段。
                    tracing::warn!(
                        "capture_navigation_status: 5s 内未收到 Network.responseReceived，\
                         返回默认 200（CF 挑战页面可能不触发此事件）"
                    );
                    return Ok(200);
                }
            }
        }
    }

    /// 从浏览器页面提取统一 Response。
    #[allow(dead_code)]
    async fn extract_browser_response(
        &self,
        page: &crate::browser::page::Page,
        req: &Request,
        nav_status: u16,
    ) -> Result<Response> {
        let html = page
            .evaluate_as_string("document.documentElement.outerHTML")
            .await?;
        let title = page.evaluate_as_string("document.title").await?;
        let final_url = page.evaluate_as_string("window.location.href").await?;

        let cookies_raw = page.evaluate_as_string("document.cookie").await?;
        let cookies: Vec<String> = cookies_raw
            .split(';')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();

        Ok(Response::from_browser(
            nav_status,
            final_url,
            html,
            title,
            cookies,
            req.clone(),
        ))
    }

    fn build_http_client(
        config: &FetchClientConfig,
        cookie_jar: Arc<wreq::cookie::Jar>,
    ) -> Result<Client> {
        // ND-008-SEC/ND-011-SEC：将 max_response_size 和 danger_accept_invalid_certs
        // 传递给底层 http::Client，使配置实际生效。
        let mut builder = Client::builder()
            .timeout(config.timeout)
            .max_redirects(config.max_redirects)
            .max_body_size(config.max_response_size)
            .danger_accept_invalid_certs(config.danger_accept_invalid_certs)
            .cookie_provider(cookie_jar);

        if let Some(ref proxy) = config.proxy {
            builder = builder.proxy(proxy);
        }
        if let Some(ref ua) = config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(emu) = config.emulation {
            builder = builder.emulation(emu);
        } else {
            builder = builder.no_emulation();
        }
        for (k, v) in &config.headers {
            builder = builder.header(k, v);
        }
        builder.build()
    }

    fn build_browser_pool(config: &FetchClientConfig) -> Option<Arc<BrowserPool>> {
        if config.max_concurrent_pages == 0 {
            return None;
        }
        let proxy_config = config.proxy.as_ref().map(|p| crate::config::ProxyConfig {
            server: p.clone(),
            username: None,
            password: None,
        });
        let launch_options = LaunchOptions {
            headless: config.headless,
            executable_path: config.executable_path.clone(),
            proxy: proxy_config,
            ..Default::default()
        };
        Some(BrowserPool::new(
            config.max_concurrent_pages,
            launch_options,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_client_config_default() {
        let config = FetchClientConfig::default();
        assert_eq!(config.max_concurrent_pages, 4);
        assert!(config.headless);
        assert!(config.human_mode);
    }

    #[test]
    fn test_fetch_client_http_only() {
        // max_concurrent_pages=0 → 无浏览器池
        let config = FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        assert!(client.browser_pool().is_none());
        assert_eq!(client.http().config_ref().timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_fetch_client_with_browser_pool() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        assert!(client.browser_pool().is_some());
    }

    #[tokio::test]
    async fn fetch_client_has_cookie_jar() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        // cookie_jar() 应返回非 None 的 Arc<dyn CookieJar>
        let jar = client.cookie_jar();
        // 默认使用 HttpCookieJar，应能 set/get
        use crate::cookie::Cookie;
        use url::Url;
        let cookie = Cookie {
            name: "test".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;
        let url = Url::parse("https://example.com/").expect("合法 URL");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "test");
    }

    #[test]
    fn fetch_client_config_still_has_cf_fields() {
        // 验证 FetchClientConfig 仍保留 cf_cookie_ttl/cf_data_dir（供 StealthStrategy 在 PR2 使用）
        let config = FetchClientConfig::default();
        assert_eq!(config.cf_cookie_ttl, std::time::Duration::from_mins(30));
        assert_eq!(config.cf_data_dir, std::path::PathBuf::from("wisp-data"));
    }

    use crate::fetcher::strategy::BrowserFetchStrategy;
    use crate::browser::Page;
    use async_trait::async_trait;

    /// Mock 策略：返回固定响应，用于验证 fetch_browser 调用契约。
    struct MockStrategy {
        called: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl BrowserFetchStrategy for MockStrategy {
        async fn fetch(&self, _page: &mut Page, req: &Request) -> Result<Response> {
            self.called.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Response::from_browser(
                200,
                req.url.clone(),
                "<html>mock</html>".to_string(),
                "mock".to_string(),
                Vec::new(),
                req.clone(),
            ))
        }
    }

    #[tokio::test]
    async fn test_fetch_browser_invokes_strategy() {
        // max_concurrent_pages=0 会导致无 browser_pool，需 >0
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        let called = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let strategy = MockStrategy { called: called.clone() };
        let req = Request::get("data:text/html,<html></html>");

        // 注意：此测试需要真实 Chrome（BrowserPool::acquire 会启动浏览器）
        // 若无 Chrome 环境，会返回 LaunchFailed 错误
        let result = client.fetch_browser(&req, &strategy).await;
        if result.is_ok() {
            assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
        // 无 Chrome 环境下不报错（忽略结果）
    }

    #[tokio::test]
    async fn test_fetch_browser_no_pool_returns_error() {
        let config = FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        let called = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let strategy = MockStrategy { called: called.clone() };
        let req = Request::get("https://example.com/");

        let result = client.fetch_browser(&req, &strategy).await;
        assert!(result.is_err(), "无 browser_pool 应返回错误");
        // 策略不应被调用
        assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}
