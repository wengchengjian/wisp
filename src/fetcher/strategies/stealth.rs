//! Stealth 模式策略：CF bypass + 人类行为模拟 + cookie 复用。
//!
//! ARCH: 从 `FetchClient::do_browser_work_inner`（solve_cf=true 分支）提取。
//! CfCookieJar 由本策略独占持有。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::browser::Page;
use crate::cookie::{CfCookieJar, CfSession};
use crate::error::{BrowserError, Result, WispError};
use crate::fetcher::client::FetchClientConfig;
use crate::fetcher::response::{Request, Response};
use crate::fetcher::strategy::{extract_browser_response, recv_navigation_status, BrowserFetchStrategy};
use crate::stealth::{ChallengeSolver, HumanBehavior, TurnstileConfig};

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
        tracing::info!("BrowserWork[+CF]: {url} 开始");

        // 启用 Network 域以捕获真实 HTTP 状态码
        page.cmd("Network.enable", serde_json::json!({}))
            .await
            .map_err(|e| {
                WispError::Browser(BrowserError::CdpConnection(format!(
                    "Network.enable failed: {e}"
                )))
            })?;

        // goto 之前订阅事件流，避免竞态
        let mut event_rx = page.session.subscribe_events();
        let sid = page.session_id.clone();

        // 注入之前保存的 CF cookie（复用 CF 挑战结果）
        let domain = url::Url::parse(&req.url)
            .ok()
            .and_then(|u| u.host_str().map(std::string::ToString::to_string));
        if let Some(ref domain) = domain {
            if let Some(session) = self.cf_jar.get_session(domain) {
                for cookie in &session.cookies {
                    let name = cookie.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let value = cookie.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let cookie_domain = cookie
                        .get("domain")
                        .and_then(|d| d.as_str())
                        .unwrap_or(domain);
                    let path = cookie.get("path").and_then(|p| p.as_str()).unwrap_or("/");
                    let secure = cookie
                        .get("secure")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let http_only = cookie
                        .get("httpOnly")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let same_site = cookie
                        .get("sameSite")
                        .and_then(|s| s.as_str())
                        .unwrap_or("Lax");
                    let _ = page
                        .cmd(
                            "Network.setCookie",
                            serde_json::json!({
                                "name": name,
                                "value": value,
                                "domain": cookie_domain,
                                "path": path,
                                "secure": secure,
                                "httpOnly": http_only,
                                "sameSite": same_site,
                            }),
                        )
                        .await;
                }
                tracing::info!(
                    "BrowserWork[+CF]: {url} 注入 {} 个 CF cookie",
                    session.cookies.len()
                );
            }
        }

        let t_nav = std::time::Instant::now();
        tracing::info!("BrowserWork[+CF]: {url} 导航");
        if let Err(e) = page.goto(&req.url).await {
            tracing::warn!("BrowserWork[+CF]: {url} goto 失败: {e}");
            return Err(e);
        }
        tracing::trace!(elapsed_ms = t_nav.elapsed().as_millis(), url = %url, "goto timing");

        // 捕获导航请求的真实 HTTP 状态码
        let t_status = std::time::Instant::now();
        let mut nav_status = match recv_navigation_status(&mut event_rx, &sid).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("BrowserWork[+CF]: {url} recv_navigation_status 失败: {e}");
                return Err(e);
            }
        };
        tracing::trace!(
            elapsed_ms = t_status.elapsed().as_millis(),
            code = nav_status,
            url = %url,
            "recv_status timing"
        );

        // CF 挑战解决
        let t_cf = std::time::Instant::now();
        let solver = ChallengeSolver::new(page);
        solver
            .solve_with_config(self.challenge_timeout, &self.turnstile)
            .await?;
        tracing::trace!(elapsed_ms = t_cf.elapsed().as_millis(), url = %url, "solve_cf timing");

        // CF 挑战解决后，nav_status 捕获的是首次 goto 时的状态码（通常是 403/503 挑战页），
        // 不能反映挑战解决后的最终页面状态。修正为 200。
        if nav_status != 200 {
            tracing::debug!(
                "BrowserWork[+CF]: {url} CF 挑战解决，状态码 {nav_status} → 200"
            );
            nav_status = 200;
        }

        // 人类行为模拟
        if self.human_mode {
            let human = HumanBehavior::new(page);
            human.random_delay(500, 1500).await?;
            human.random_scroll().await?;
            human.random_delay(300, 800).await?;
        }

        // CF 挑战解决后，保存 cookie + 浏览器实际 UA 到缓存
        if let Some(ref domain) = domain {
            let mut ua_str = String::new();
            if let Ok(ua_val) = page.evaluate("navigator.userAgent").await {
                if let Some(s) = ua_val.as_str() {
                    ua_str = s.to_string();
                }
            }
            if let Ok(resp) = page.cmd("Network.getCookies", serde_json::json!({})).await {
                if let Some(cookies) = resp.pointer("/cookies").and_then(|c| c.as_array()) {
                    let cookies_to_save: Vec<serde_json::Value> = cookies
                        .iter()
                        .filter(|c| {
                            let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            // 只保存 CF 相关 cookie（cf_clearance 等）
                            name.starts_with("cf_") || name.starts_with("__cf")
                        })
                        .cloned()
                        .collect();
                    if !cookies_to_save.is_empty() {
                        self.cf_jar.insert_session(
                            domain.clone(),
                            CfSession {
                                cookies: cookies_to_save.clone(),
                                ua: ua_str,
                                saved_at: chrono::Utc::now().timestamp(),
                            },
                        );
                        tracing::info!(
                            "BrowserWork[+CF]: {url} 保存 {} 个 CF cookie",
                            cookies_to_save.len()
                        );
                    }
                }
            }
        }

        // 等待特定选择器
        if let Some(ref selector) = self.wait_for {
            page.wait_for_selector(selector, self.timeout.as_millis() as u64)
                .await?;
        }

        // 额外等待
        if self.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.extra_wait_ms)).await;
        }

        tracing::debug!("BrowserWork[+CF]: {url} 提取响应");
        let resp = extract_browser_response(page, req, nav_status).await?;
        tracing::info!(
            "BrowserWork[+CF]: {url} 完成 ({} bytes)",
            resp.body.len()
        );
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config_default() {
        let config = FetchClientConfig::default();
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        let strategy = StealthStrategy::from_config(&config, cf_jar);
        assert_eq!(strategy.challenge_timeout, config.challenge_timeout);
        assert_eq!(strategy.human_mode, config.human_mode);
        assert_eq!(strategy.wait_for, config.wait_for);
        assert_eq!(strategy.extra_wait_ms, config.extra_wait_ms);
    }

    #[test]
    fn test_from_config_custom() {
        let config = FetchClientConfig {
            human_mode: false,
            challenge_timeout: Duration::from_secs(60),
            wait_for: Some(".loaded".to_string()),
            extra_wait_ms: 2000,
            ..Default::default()
        };
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        let strategy = StealthStrategy::from_config(&config, cf_jar);
        assert!(!strategy.human_mode);
        assert_eq!(strategy.challenge_timeout, Duration::from_secs(60));
        assert_eq!(strategy.wait_for.as_deref(), Some(".loaded"));
        assert_eq!(strategy.extra_wait_ms, 2000);
    }

    /// 集成测试：CF 挑战解决 + cookie 持久化。
    /// 运行方式：cargo test --lib fetcher::strategies::stealth -- --ignored
    #[tokio::test]
    #[ignore = "需要 CF 保护的站点环境"]
    async fn test_stealth_strategy_solves_cf() {
        use crate::browser::BrowserPool;
        use crate::config::LaunchOptions;
        use crate::fetcher::response::Request;

        let config = FetchClientConfig::default();
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        let strategy = StealthStrategy::from_config(&config, cf_jar);

        let pool = BrowserPool::new(1, LaunchOptions::default());
        let mut handle = pool.acquire().await.expect("acquire page");
        let page = handle.page_mut();

        // 替换为实际的 CF 保护站点
        let req = Request::get("https://example.com/");
        let resp = strategy.fetch(page, &req).await.expect("fetch 应成功");
        assert_eq!(resp.status, 200);
    }
}
