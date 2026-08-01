//! Stealth 模式策略：CF bypass + 人类行为模拟 + cookie 复用。
//!
//! ARCH: 从 `FetchClient::do_browser_work_inner`（solve_cf=true 分支）提取。
//! CfCookieJar 由本策略独占持有。

mod cookie;
mod extract;
mod navigation;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::client::FetchClientConfig;
use crate::cookie::CfCookieJar;
use crate::strategy::BrowserFetchStrategy;
use wisp_browser::Page;
use wisp_core::error::Result;
use wisp_core::{Request, Response};
use wisp_stealth::TurnstileConfig;

/// Stealth 模式策略：CF bypass + 人类行为模拟 + cookie 复用。
///
/// ARCH: CfCookieJar 从 FetchClient 迁入此策略，由 StealthStrategy 独占持有。
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
    /// CF 会话 cookie 缓存（moka + 文件持久化）。
    cf_jar: Arc<CfCookieJar>,
}

impl StealthStrategy {
    /// 从 FetchClientConfig + 共享 CfCookieJar 构造。
    pub fn from_config(config: &FetchClientConfig, cf_jar: Arc<CfCookieJar>) -> Self {
        Self {
            challenge_timeout: config.challenge_timeout,
            turnstile: config.turnstile.clone(),
            human_mode: config.human_mode,
            wait_for: config.wait_for.clone(),
            extra_wait_ms: config.extra_wait_ms,
            timeout: config.timeout,
            cf_jar,
        }
    }
}

#[async_trait]
impl BrowserFetchStrategy for StealthStrategy {
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response> {
        let url = &req.url;
        Self::enable_network(page, url).await?;

        let mut event_rx = page.session().subscribe_events();
        let sid = page.session_id().to_string();

        let domain = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(std::string::ToString::to_string));
        self.inject_cf_cookies(page, domain.as_deref(), url).await;

        let nav_status = Self::navigate_and_capture_status(page, url, &mut event_rx, &sid).await?;
        let nav_status = self.solve_cf(page, url, nav_status).await?;
        self.simulate_human(page).await?;
        self.persist_cf_session(page, domain.as_deref(), url).await;
        self.wait_and_extract(page, req, nav_status).await
    }
}
