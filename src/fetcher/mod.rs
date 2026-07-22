//! 统一 Fetcher 入口 - 根据模式自动选择底层实现。
//!
//! # 三模式
//!
//! - `FetchMode::Http` - 快速 HTTP（TLS 指纹模拟，毫秒级，无浏览器）
//! - `FetchMode::Dynamic` - 浏览器渲染（JS 执行，秒级）
//! - `FetchMode::Stealth` - 隐身浏览器（CF bypass + 人类行为模拟，秒级）
//!
//! # 示例
//!
//! ```rust,no_run
//! use wisp::Fetcher;
//!
//! # async fn example() -> wisp::Result<()> {
//! // 三模式，统一 API
//! let page = Fetcher::http().get("https://quotes.toscrape.com/").await?;
//! let quotes = page.css(".quote");
//!
//! let page = Fetcher::stealth()
//!     .proxy("http://127.0.0.1:7897")
//!     .get("https://cf-protected.com/")
//!     .await?;
//! let data = page.css(".content");
//! # Ok(())
//! # }
//! ```

pub mod response;
pub mod session;

pub use response::{Response, Request, Method};
pub use session::Session;

use std::collections::HashMap;
use std::time::Duration;
use wreq_util::Profile;

use crate::error::Result;
use crate::http::block::DomainBlocker;

/// 抓取模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    /// 快速 HTTP（TLS 指纹模拟，无浏览器）。成本最低，毫秒级。
    Http,
    /// 浏览器渲染（JS 执行，无 CF 绕过）。中等成本，秒级。
    Dynamic,
    /// 隐身浏览器（CF bypass + 人类行为模拟）。最高成本，秒级。
    Stealth,
    /// 自动模式：先 HTTP，被拦截升级 Stealth，选择器无内容升级 Dynamic。
    /// 仅 Spider/Engine 场景支持。
    Auto,
}

/// Fetcher 配置。
#[derive(Debug, Clone)]
pub struct FetcherConfig {
    /// 请求超时
    pub timeout: Duration,
    /// 代理 URL
    pub proxy: Option<String>,
    /// 浏览器 headless 模式
    pub headless: bool,
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
}

impl Default for FetcherConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            proxy: None,
            headless: true,
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
        }
    }
}

/// 统一 Fetcher - 根据模式自动选择底层实现。
pub struct Fetcher {
    mode: FetchMode,
    config: FetcherConfig,
}

impl Fetcher {
    /// 快速 HTTP 模式（TLS 指纹，毫秒级）。
    pub fn http() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Http)
    }

    /// 浏览器渲染模式（JS 执行，秒级）。
    pub fn dynamic() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Dynamic)
    }

    /// 隐身模式（CF bypass，秒级）。
    pub fn stealth() -> FetcherBuilder {
        FetcherBuilder::new(FetchMode::Stealth)
    }

    /// 从已有配置创建 Fetcher。
    pub fn new(mode: FetchMode, config: FetcherConfig) -> Self {
        Self { mode, config }
    }

    /// 获取当前模式。
    pub fn mode(&self) -> FetchMode {
        self.mode
    }

    /// 获取配置引用。
    pub fn config(&self) -> &FetcherConfig {
        &self.config
    }

    /// GET 请求。
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.fetch(Request::get(url)).await
    }

    /// POST 请求。
    pub async fn post(&self, url: &str, body: Option<&str>) -> Result<Response> {
        let mut req = Request::post(url, body.map(|b| b.to_string()));
        req.headers = self.config.headers.clone();
        self.fetch(req).await
    }

    /// 发送请求（根据模式分发到不同底层实现）。
    pub async fn fetch(&self, req: Request) -> Result<Response> {
        match self.mode {
            FetchMode::Http => self.fetch_http(req).await,
            FetchMode::Dynamic => self.fetch_dynamic(req).await,
            FetchMode::Stealth => self.fetch_stealth(req).await,
            // Auto 模式仅 Spider/Engine 支持，直接调用时回退到 Http
            FetchMode::Auto => self.fetch_http(req).await,
        }
    }

    // === Http 模式：wreq TLS 指纹模拟 ===
    async fn fetch_http(&self, req: Request) -> Result<Response> {
        let mut builder = crate::http::Client::builder()
            .timeout(self.config.timeout)
            .max_redirects(self.config.max_redirects);

        if let Some(ref proxy) = self.config.proxy {
            builder = builder.proxy(proxy);
        }
        if let Some(ref ua) = self.config.user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(emu) = self.config.emulation {
            builder = builder.emulation(emu);
        } else {
            builder = builder.no_emulation();
        }
        for (k, v) in &self.config.headers {
            builder = builder.header(k, v);
        }

        let client = builder.build()?;

        let resp = match req.method {
            Method::Get => client.get(&req.url).await?,
            Method::Post => client.post(&req.url, req.body.as_deref(), None).await?,
            Method::Put => client.put(&req.url, req.body.as_deref(), None).await?,
            Method::Delete => client.delete(&req.url).await?,
        };

        Ok(Response::from_http(
            resp.status,
            resp.url.clone(),
            resp.headers.clone(),
            resp.body.clone(),
            resp.headers.get("content-type").cloned().unwrap_or_default(),
            Some(req),
        ))
    }

    // === Dynamic 模式：浏览器渲染 ===
    async fn fetch_dynamic(&self, req: Request) -> Result<Response> {
        self.fetch_browser_page(req, false).await
    }

    // === Stealth 模式：CF bypass + 人类行为 ===
    async fn fetch_stealth(&self, req: Request) -> Result<Response> {
        self.fetch_browser_page(req, true).await
    }

    /// 浏览器模式共用逻辑（Dynamic/Stealth 合并）。
    /// `solve_cf=true` 时额外执行 CF 挑战解决 + 人类行为模拟。
    async fn fetch_browser_page(&self, req: Request, solve_cf: bool) -> Result<Response> {
        let browser = self.launch_browser().await?;
        let mut page = browser.new_page().await?;

        // 启用 Network 域以捕获真实 HTTP 状态码
        let _ = page.cmd("Network.enable", serde_json::json!({})).await;

        page.goto(&req.url).await?;

        // 通过 CDP Network.responseReceived 获取真实状态码
        let nav_status = self.capture_navigation_status(&page).await;

        if solve_cf {
            // 检测并解决 Cloudflare 挑战
            let solver = crate::stealth::challenge::ChallengeSolver::new(&page);
            solver.solve(self.config.challenge_timeout).await?;

            // 人类行为模拟
            if self.config.human_mode {
                let human = crate::stealth::human::HumanBehavior::new(&page);
                human.random_delay(500, 1500).await?;
                human.random_scroll().await?;
                human.random_delay(300, 800).await?;
            }
        }

        // 等待特定选择器
        if let Some(ref selector) = self.config.wait_for {
            page.wait_for_selector(selector, self.config.timeout.as_millis() as u64).await?;
        }

        // 额外等待
        if self.config.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.config.extra_wait_ms)).await;
        }

        let result = self.extract_browser_response(&page, &req, nav_status).await;
        browser.close().await?;
        result
    }

    /// 从 CDP Network.responseReceived 事件中捕获导航请求的真实 HTTP 状态码。
    /// 返回 None 表示无法获取（超时或事件缺失）。
    async fn capture_navigation_status(&self, page: &crate::browser::page::Page) -> Option<u16> {
        let sid = page.session_id.clone();
        let event = page.session.wait_for_event(
            move |e| {
                if e.method == "Network.responseReceived" {
                    let is_doc = e.params.get("type").and_then(|t| t.as_str()) == Some("Document");
                    let match_session = e.session_id.as_deref() == Some(sid.as_str()) || e.session_id.is_none();
                    is_doc && match_session
                } else {
                    false
                }
            },
            5000,
        ).await;

        event.ok().and_then(|e| {
            e.params.get("response")
                .and_then(|r| r.get("status"))
                .and_then(|s| s.as_u64())
                .map(|s| s as u16)
        })
    }

    // === 内部辅助 ===

    /// 启动浏览器（Dynamic/Stealth 共用）。
    async fn launch_browser(&self) -> Result<crate::browser::Browser> {
        let proxy_config = self.config.proxy.as_ref().map(|p| crate::config::ProxyConfig {
            server: p.clone(),
            username: None,
            password: None,
        });

        crate::browser::Browser::launch(crate::config::LaunchOptions {
            headless: self.config.headless,
            proxy: proxy_config,
            ..Default::default()
        }).await
    }

    /// 从浏览器页面提取统一 Response。
    ///
    /// 状态码优先使用 CDP Network.responseReceived 获取的真实值，
    /// 仅在无法获取时 fallback 到 <title> 精确匹配。
    async fn extract_browser_response(
        &self,
        page: &crate::browser::page::Page,
        req: &Request,
        nav_status: Option<u16>,
    ) -> Result<Response> {
        let html = page.evaluate_as_string("document.documentElement.outerHTML").await?;
        let title = page.evaluate_as_string("document.title").await?;
        let final_url = page.evaluate_as_string("window.location.href").await?;

        let cookies_raw = page.evaluate_as_string("document.cookie").await?;
        let cookies: Vec<String> = cookies_raw.split(';')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();

        // 状态码：优先用 CDP 真实值，fallback 到 <title> 精确匹配
        let status = if let Some(code) = nav_status {
            code
        } else {
            let title_lower = title.to_lowercase();
            if title_lower.contains("403") && title_lower.contains("forbidden") {
                403
            } else if title_lower.contains("404") && title_lower.contains("not found") {
                404
            } else {
                200
            }
        };

        Ok(Response::from_browser(
            status,
            final_url,
            html,
            title,
            cookies,
            Some(req.clone()),
        ))
    }
}

/// Fetcher 构建器 - 链式配置。
pub struct FetcherBuilder {
    mode: FetchMode,
    config: FetcherConfig,
}

impl FetcherBuilder {
    pub(crate) fn new(mode: FetchMode) -> Self {
        Self { mode, config: FetcherConfig::default() }
    }

    /// 设置代理。
    pub fn proxy(mut self, url: &str) -> Self {
        self.config.proxy = Some(url.to_string());
        self
    }

    /// 设置超时。
    pub fn timeout(mut self, d: Duration) -> Self {
        self.config.timeout = d;
        self
    }

    /// 设置 headless 模式（浏览器模式）。
    pub fn headless(mut self, v: bool) -> Self {
        self.config.headless = v;
        self
    }

    /// 设置 TLS 指纹模拟（Http 模式）。
    pub fn emulation(mut self, p: Profile) -> Self {
        self.config.emulation = Some(p);
        self
    }

    /// 关闭 TLS 指纹模拟。
    pub fn no_emulation(mut self) -> Self {
        self.config.emulation = None;
        self
    }

    /// 设置 User-Agent。
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.config.user_agent = Some(ua.to_string());
        self
    }

    /// 添加自定义 header。
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.config.headers.insert(key.to_string(), value.to_string());
        self
    }

    /// 启用/禁用人类行为模拟（Stealth 模式）。
    pub fn human_mode(mut self, v: bool) -> Self {
        self.config.human_mode = v;
        self
    }

    /// 设置 CF 挑战超时（Stealth 模式）。
    pub fn challenge_timeout(mut self, d: Duration) -> Self {
        self.config.challenge_timeout = d;
        self
    }

    /// 等待特定 CSS 选择器出现（浏览器模式）。
    pub fn wait_for(mut self, selector: &str) -> Self {
        self.config.wait_for = Some(selector.to_string());
        self
    }

    /// 页面加载后额外等待（毫秒）。
    pub fn extra_wait_ms(mut self, ms: u64) -> Self {
        self.config.extra_wait_ms = ms;
        self
    }

    /// 启用内置广告拦截。
    pub fn block_ads(mut self) -> Self {
        let mut blocker = self.config.domain_blocker.take().unwrap_or_default();
        blocker.enable_ad_blocking();
        self.config.domain_blocker = Some(blocker);
        self
    }

    /// 拦截指定域名。
    pub fn block_domains(mut self, domains: &[&str]) -> Self {
        let mut blocker = self.config.domain_blocker.take().unwrap_or_default();
        blocker.block_domains(domains);
        self.config.domain_blocker = Some(blocker);
        self
    }

    /// 设置 DNS-over-HTTPS。
    pub fn dns_over_https(mut self, url: &str) -> Self {
        self.config.dns_over_https = Some(url.to_string());
        self
    }

    /// 设置最大重定向次数。
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.config.max_redirects = n;
        self
    }

    /// 构建 Fetcher 实例。
    pub fn build(self) -> Fetcher {
        Fetcher { mode: self.mode, config: self.config }
    }

    /// 快捷方式：build + get。
    pub async fn get(self, url: &str) -> Result<Response> {
        self.build().get(url).await
    }

    /// 快捷方式：build + post。
    pub async fn post(self, url: &str, body: Option<&str>) -> Result<Response> {
        self.build().post(url, body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_mode_enum() {
        assert_ne!(FetchMode::Http, FetchMode::Dynamic);
        assert_ne!(FetchMode::Dynamic, FetchMode::Stealth);
        assert_eq!(FetchMode::Http, FetchMode::Http);
    }

    #[test]
    fn test_fetcher_builder_http() {
        let fetcher = Fetcher::http()
            .proxy("http://127.0.0.1:7897")
            .timeout(Duration::from_secs(60))
            .emulation(Profile::Firefox128)
            .build();

        assert_eq!(fetcher.mode(), FetchMode::Http);
        assert_eq!(fetcher.config().proxy.as_deref(), Some("http://127.0.0.1:7897"));
        assert_eq!(fetcher.config().timeout, Duration::from_secs(60));
        assert_eq!(fetcher.config().emulation, Some(Profile::Firefox128));
    }

    #[test]
    fn test_fetcher_builder_stealth() {
        let fetcher = Fetcher::stealth()
            .headless(true)
            .human_mode(true)
            .challenge_timeout(Duration::from_secs(60))
            .proxy("http://127.0.0.1:7897")
            .build();

        assert_eq!(fetcher.mode(), FetchMode::Stealth);
        assert!(fetcher.config().headless);
        assert!(fetcher.config().human_mode);
        assert_eq!(fetcher.config().challenge_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_fetcher_builder_dynamic() {
        let fetcher = Fetcher::dynamic()
            .headless(false)
            .wait_for(".content")
            .extra_wait_ms(2000)
            .build();

        assert_eq!(fetcher.mode(), FetchMode::Dynamic);
        assert!(!fetcher.config().headless);
        assert_eq!(fetcher.config().wait_for.as_deref(), Some(".content"));
        assert_eq!(fetcher.config().extra_wait_ms, 2000);
    }

    #[test]
    fn test_fetcher_builder_block_ads() {
        let fetcher = Fetcher::dynamic()
            .block_ads()
            .block_domains(&["analytics.example.com"])
            .build();

        let blocker = fetcher.config().domain_blocker.as_ref().unwrap();
        assert!(blocker.is_ad_blocking_enabled());
        assert!(blocker.should_block("https://analytics.example.com/track"));
    }

    #[test]
    fn test_fetcher_config_default() {
        let config = FetcherConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.headless);
        assert!(config.human_mode);
        assert_eq!(config.emulation, Some(Profile::Chrome136));
        assert!(config.proxy.is_none());
        assert!(config.domain_blocker.is_none());
    }
}
