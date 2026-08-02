//! Fetcher builder — 链式配置一次性请求客户端。

use crate::client::FetchClientConfig;
use crate::Fetcher;
use std::time::Duration;

use wisp_core::error::Result;
use wisp_core::{FetchMode, Response};
use wreq_util::Profile;

/// Fetcher 构建器 - 链式配置。
pub struct FetcherBuilder {
    mode: FetchMode,
    config: FetchClientConfig,
}

impl FetcherBuilder {
    pub(crate) fn new(mode: FetchMode) -> Self {
        let mut config = FetchClientConfig::default();
        if matches!(mode, FetchMode::Stealth) {
            config.force_headed_offscreen = true;
        }
        Self { mode, config }
    }

    /// 设置代理。
    #[must_use]
    pub fn proxy(mut self, url: &str) -> Self {
        self.config.proxy = Some(url.to_string());
        self
    }

    /// 设置超时。
    #[must_use]
    pub fn timeout(mut self, d: Duration) -> Self {
        self.config.timeout = d;
        self
    }

    /// 设置 headless 模式（浏览器模式）。
    /// Stealth 模式会临时使用 headed + offscreen 真实渲染，其他模式按此值执行。
    #[must_use]
    pub fn headless(mut self, v: bool) -> Self {
        self.config.headless = v;
        self
    }

    /// 设置 TLS 指纹模拟（Http 模式）。
    #[must_use]
    pub fn emulation(mut self, p: Profile) -> Self {
        self.config.emulation = Some(p);
        self
    }

    /// 关闭 TLS 指纹模拟。
    #[must_use]
    pub fn no_emulation(mut self) -> Self {
        self.config.emulation = None;
        self
    }

    /// 设置 User-Agent。
    #[must_use]
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.config.user_agent = Some(ua.to_string());
        self
    }

    /// 添加自定义 header。
    #[must_use]
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.config
            .headers
            .insert(key.to_string(), value.to_string());
        self
    }

    /// 启用/禁用人类行为模拟（Stealth 模式）。
    #[must_use]
    pub fn human_mode(mut self, v: bool) -> Self {
        self.config.human_mode = v;
        self
    }

    /// 设置 CF 挑战超时（Stealth 模式）。
    #[must_use]
    pub fn challenge_timeout(mut self, d: Duration) -> Self {
        self.config.challenge_timeout = d;
        self
    }

    /// 设置 Turnstile 解决器参数。
    #[must_use]
    #[cfg(feature = "stealth")]
    pub fn turnstile_config(mut self, cfg: wisp_stealth::TurnstileConfig) -> Self {
        self.config.turnstile = cfg;
        self
    }

    /// 等待特定 CSS 选择器出现（浏览器模式）。
    #[must_use]
    pub fn wait_for(mut self, selector: &str) -> Self {
        self.config.wait_for = Some(selector.to_string());
        self
    }

    /// 页面加载后额外等待（毫秒）。
    #[must_use]
    pub fn extra_wait_ms(mut self, ms: u64) -> Self {
        self.config.extra_wait_ms = ms;
        self
    }

    /// 启用内置广告拦截。
    #[must_use]
    pub fn block_ads(mut self) -> Self {
        let mut blocker = self.config.domain_blocker.take().unwrap_or_default();
        blocker.enable_ad_blocking();
        self.config.domain_blocker = Some(blocker);
        self
    }

    /// 拦截指定域名。
    #[must_use]
    pub fn block_domains(mut self, domains: &[&str]) -> Self {
        let mut blocker = self.config.domain_blocker.take().unwrap_or_default();
        blocker.block_domains(domains);
        self.config.domain_blocker = Some(blocker);
        self
    }

    /// 设置 BrowserPool 最大并发 page 数（0 = 禁用浏览器模式）。
    #[must_use]
    pub fn max_concurrent_pages(mut self, size: usize) -> Self {
        self.config.max_concurrent_pages = size;
        self
    }

    /// 设置 DNS-over-HTTPS。
    #[must_use]
    pub fn dns_over_https(mut self, url: &str) -> Self {
        self.config.dns_over_https = Some(url.to_string());
        self
    }

    /// 设置最大重定向次数。
    #[must_use]
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.config.max_redirects = n;
        self
    }

    /// 构建 Fetcher 实例。
    ///
    /// # Errors
    ///
    /// HTTP Client 构建失败时返回错误（如无效代理 URL）。
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
