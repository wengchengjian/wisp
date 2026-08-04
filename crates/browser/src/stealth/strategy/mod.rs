//! Stealth 模式策略：CF bypass + 人类行为模拟 + cookie 复用。
//!
//! ARCH: 从 `FetchClient::do_browser_work_inner`（solve_cf=true 分支）提取。
//! CfCookieJar 由本策略独占持有；导航由 `Page::goto` 统一负责。
//! 本策略归属 browser 领域，配置通过 [`StealthConfig`] 注入，不依赖高层的
//! `FetchClientConfig`（避免 browser → fetcher 逆依赖）。

mod cookie;
mod extract;
mod navigation;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::extract::extract_browser_response;
use crate::strategy::BrowserFetchStrategy;
use crate::Page;
use wisp_core::cookie::CookieJar;
use wisp_core::error::Result;
use wisp_core::stealth::TurnstileConfig;
use wisp_core::{Request, Response};

/// Stealth 策略的浏览器领域配置。
///
/// 由 fetcher 的 `FetchClientConfig` 转换而来，避免 browser 依赖 fetcher。
#[derive(Debug, Clone)]
pub struct StealthConfig {
    /// CF 挑战超时。
    pub challenge_timeout: Duration,
    /// Turnstile 解决器参数。
    pub turnstile: TurnstileConfig,
    /// 是否启用人类行为模拟。
    pub human_mode: bool,
    /// 等待特定 CSS 选择器出现（可选）。
    pub wait_for: Option<String>,
    /// 页面加载后额外等待（毫秒）。
    pub extra_wait_ms: u64,
    /// 单操作超时（用于 wait_for_selector）。
    pub timeout: Duration,
}

impl Default for StealthConfig {
    fn default() -> Self {
        Self {
            challenge_timeout: Duration::from_secs(30),
            turnstile: TurnstileConfig::default(),
            human_mode: true,
            wait_for: None,
            extra_wait_ms: 0,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Stealth 模式策略：CF bypass + 人类行为模拟 + cookie 复用。
pub struct StealthStrategy {
    /// CF 挑战超时。
    challenge_timeout: Duration,
    /// Turnstile 解决器参数。
    turnstile: TurnstileConfig,
    /// 是否启用人类行为模拟。
    human_mode: bool,
    /// 等待特定 CSS 选择器出现（可选）。
    wait_for: Option<String>,
    /// 页面加载后额外等待（毫秒）。
    extra_wait_ms: u64,
    /// 单操作超时（用于 wait_for_selector）。
    timeout: Duration,
    /// 共享 cookie seam（HTTP/CF 复合状态）。
    cookie_jar: Arc<dyn CookieJar>,
}

impl StealthStrategy {
    /// 从 StealthConfig + 共享 CfCookieJar 构造。
    pub fn from_config(config: &StealthConfig, cookie_jar: Arc<dyn CookieJar>) -> Self {
        Self {
            challenge_timeout: config.challenge_timeout,
            turnstile: config.turnstile.clone(),
            human_mode: config.human_mode,
            wait_for: config.wait_for.clone(),
            extra_wait_ms: config.extra_wait_ms,
            timeout: config.timeout,
            cookie_jar,
        }
    }
}

#[async_trait]
impl BrowserFetchStrategy for StealthStrategy {
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response> {
        let url = &req.url;
        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(std::string::ToString::to_string));
        self.inject_cf_cookies(page, domain.as_deref(), url).await;
        let nav_status = page.goto(url).await?;
        let nav_status = self.solve_cf(page, url, nav_status).await?;
        self.simulate_human(page).await?;
        self.persist_cf_session(page, domain.as_deref(), url).await;
        self.wait_and_extract(page, req, nav_status).await
    }
}