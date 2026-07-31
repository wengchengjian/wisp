//! Dynamic 模式策略：浏览器渲染 + JS 执行，无 CF 绕过。

use std::time::Duration;

use async_trait::async_trait;

use wisp_browser::Page;
use wisp_core::error::{BrowserError, Result, WispError};
use crate::client::FetchClientConfig;
use wisp_core::{Request, Response};
use crate::strategy::{extract_browser_response, recv_navigation_status, BrowserFetchStrategy};

/// Dynamic 模式策略：浏览器渲染 + JS 执行，无 CF 绕过。
///
/// ARCH: 从 `FetchClient::do_browser_work_inner`（solve_cf=false 分支）提取。
/// 仅做导航 + 等待选择器 + 提取响应。
pub struct DynamicStrategy {
    /// 等待特定 CSS 选择器出现（可选）。
    wait_for: Option<String>,
    /// 页面加载后额外等待（毫秒）。
    extra_wait_ms: u64,
    /// 单操作超时（用于 wait_for_selector）。
    timeout: Duration,
}

impl DynamicStrategy {
    /// 从 FetchClientConfig 构造。
    pub fn from_config(config: &FetchClientConfig) -> Self {
        Self {
            wait_for: config.wait_for.clone(),
            extra_wait_ms: config.extra_wait_ms,
            timeout: config.timeout,
        }
    }
}

#[async_trait]
impl BrowserFetchStrategy for DynamicStrategy {
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response> {
        let url = &req.url;
        tracing::info!("BrowserWork: {url} 开始（Dynamic）");

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

        let t_nav = std::time::Instant::now();
        tracing::info!("BrowserWork: {url} 导航");
        if let Err(e) = page.goto(&req.url).await {
            tracing::warn!("BrowserWork: {url} goto 失败: {e}");
            return Err(e);
        }
        tracing::trace!(elapsed_ms = t_nav.elapsed().as_millis(), url = %url, "goto timing");

        // 捕获导航请求的真实 HTTP 状态码
        let t_status = std::time::Instant::now();
        let nav_status = match recv_navigation_status(&mut event_rx, &sid).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("BrowserWork: {url} recv_navigation_status 失败: {e}");
                return Err(e);
            }
        };
        tracing::trace!(
            elapsed_ms = t_status.elapsed().as_millis(),
            code = nav_status,
            url = %url,
            "recv_status timing"
        );

        // 等待特定选择器
        if let Some(ref selector) = self.wait_for {
            page.wait_for_selector(selector, self.timeout.as_millis() as u64)
                .await?;
        }

        // 额外等待
        if self.extra_wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.extra_wait_ms)).await;
        }

        tracing::debug!("BrowserWork: {url} 提取响应");
        let resp = extract_browser_response(page, req, nav_status).await?;
        tracing::info!("BrowserWork: {url} 完成 ({} bytes)", resp.body.len());
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config_default() {
        let config = FetchClientConfig::default();
        let strategy = DynamicStrategy::from_config(&config);
        assert_eq!(strategy.wait_for, config.wait_for);
        assert_eq!(strategy.extra_wait_ms, config.extra_wait_ms);
        assert_eq!(strategy.timeout, config.timeout);
    }

    #[test]
    fn test_from_config_with_wait_for() {
        let config = FetchClientConfig {
            wait_for: Some(".content".to_string()),
            extra_wait_ms: 1500,
            ..Default::default()
        };
        let strategy = DynamicStrategy::from_config(&config);
        assert_eq!(strategy.wait_for.as_deref(), Some(".content"));
        assert_eq!(strategy.extra_wait_ms, 1500);
    }

    /// 集成测试：实际浏览器导航 + wait_for_selector。
    /// 运行方式：cargo test --lib fetcher::strategies::dynamic -- --ignored
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_dynamic_strategy_navigates() {
        use wisp_browser::BrowserPool;
        use wisp_core::config::LaunchOptions;
        use wisp_core::Request;

        let pool = BrowserPool::new(1, LaunchOptions::default());
        let mut handle = pool.acquire().await.expect("acquire page");
        let page = handle.page_mut();

        let strategy = DynamicStrategy {
            wait_for: None,
            extra_wait_ms: 0,
            timeout: Duration::from_secs(30),
        };
        let req = Request::get("data:text/html,<html><body><h1>Test</h1></body></html>");
        let resp = strategy.fetch(page, &req).await.expect("fetch 应成功");
        assert_eq!(resp.status, 200);
        assert!(resp.body.len() > 0);
    }
}
