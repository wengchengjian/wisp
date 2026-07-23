//! 统一 Fetcher 入口 - 根据 FetchMode 委托给 FetchClient。
//!
//! Fetcher 是 FetchClient 的薄包装，用于一次性请求场景。
//! 持续爬取场景应直接使用 FetchClient。
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

pub mod client;
pub mod response;

pub use client::{FetchClient, FetchClientConfig};
pub use response::{Method, Request, Response};

use std::sync::Arc;
use std::time::Duration;
use wreq_util::Profile;

use crate::error::Result;

/// 抓取模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    /// 快速 HTTP（TLS 指纹模拟，无浏览器）。成本最低，毫秒级。
    Http,
    /// 浏览器渲染（JS 执行，无 CF 绕过）。中等成本，秒级。
    Dynamic,
    /// 隐身浏览器（CF bypass + 人类行为模拟）。最高成本，秒级。
    Stealth,
    /// 自动模式：先 HTTP，中间件驱动升级（DynamicUpgradeMiddleware / StealthUpgradeMiddleware）。
    /// 仅 Spider/Engine 场景支持。
    Auto,
}

/// Fetcher — FetchClient 的薄包装，用于一次性请求场景。
///
/// 持有 `Arc<FetchClient>`，所有请求委托给 FetchClient。
/// HTTP 请求共享连接池，浏览器请求通过 BrowserPool 复用实例。
pub struct Fetcher {
    client: Arc<FetchClient>,
    mode: FetchMode,
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

    /// 从已有 FetchClient 创建 Fetcher。
    pub fn from_client(client: Arc<FetchClient>, mode: FetchMode) -> Self {
        Self { client, mode }
    }

    /// 从配置创建 Fetcher。
    pub fn new(mode: FetchMode, config: FetchClientConfig) -> Result<Self> {
        Ok(Self {
            client: Arc::new(FetchClient::new(config)?),
            mode,
        })
    }

    /// 获取当前模式。
    pub fn mode(&self) -> FetchMode {
        self.mode
    }

    /// 获取底层 FetchClient 引用。
    pub fn client(&self) -> &FetchClient {
        &self.client
    }

    /// 获取配置引用。
    pub fn config(&self) -> &FetchClientConfig {
        self.client.config()
    }

    /// GET 请求。
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.fetch(Request::get(url)).await
    }

    /// POST 请求。
    pub async fn post(&self, url: &str, body: Option<&str>) -> Result<Response> {
        let mut req = Request::post(url, body.map(|b| b.to_string()));
        req.headers = self.config().headers.clone();
        self.fetch(req).await
    }

    /// 发送请求（根据模式委托给 FetchClient）。
    pub async fn fetch(&self, req: Request) -> Result<Response> {
        match self.mode {
            FetchMode::Http | FetchMode::Auto => self.client.fetch_http(&req).await,
            FetchMode::Dynamic => self.client.fetch_browser(&req, false).await,
            FetchMode::Stealth => self.client.fetch_browser(&req, true).await,
        }
    }
}

/// Fetcher 构建器 - 链式配置。
pub struct FetcherBuilder {
    mode: FetchMode,
    config: FetchClientConfig,
}

impl FetcherBuilder {
    pub(crate) fn new(mode: FetchMode) -> Self {
        Self {
            mode,
            config: FetchClientConfig::default(),
        }
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

    /// 设置 BrowserPool 最大并发 page 数（0 = 禁用浏览器模式）。
    pub fn max_concurrent_pages(mut self, size: usize) -> Self {
        self.config.max_concurrent_pages = size;
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
    pub fn build(self) -> Result<Fetcher> {
        Fetcher::new(self.mode, self.config)
    }

    /// 快捷方式：build + get。
    pub async fn get(self, url: &str) -> Result<Response> {
        self.build()?.get(url).await
    }

    /// 快捷方式：build + post。
    pub async fn post(self, url: &str, body: Option<&str>) -> Result<Response> {
        self.build()?.post(url, body).await
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
            .build()
            .expect("build fetcher");

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
            .build()
            .expect("build fetcher");

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
            .build()
            .expect("build fetcher");

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
            .build()
            .expect("build fetcher");

        let blocker = fetcher.config().domain_blocker.as_ref().unwrap();
        assert!(blocker.is_ad_blocking_enabled());
        assert!(blocker.should_block("https://analytics.example.com/track"));
    }

    #[test]
    fn test_fetcher_config_default() {
        let config = FetchClientConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.headless);
        assert!(config.human_mode);
        assert_eq!(config.emulation, Some(Profile::Chrome136));
        assert!(config.proxy.is_none());
        assert!(config.domain_blocker.is_none());
    }
}
