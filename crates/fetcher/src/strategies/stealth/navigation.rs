//! 浏览器导航、CF 挑战解决与人类行为模拟。

use super::*;

use crate::strategy::recv_navigation_status;
use wisp_core::error::{BrowserError, Result, WispError};
use wisp_stealth::{ChallengeSolver, HumanBehavior};

impl StealthStrategy {
    pub(super) async fn enable_network(page: &mut Page, url: &str) -> Result<()> {
        page.cmd("Network.enable", serde_json::json!({}))
            .await
            .map_err(|e| {
                WispError::Browser(BrowserError::CdpConnection(format!(
                    "Network.enable failed: {e}"
                )))
            })?;
        tracing::info!("BrowserWork[+CF]: {url} 开始");
        Ok(())
    }

    pub(super) async fn navigate_and_capture_status(
        page: &mut Page,
        url: &str,
        event_rx: &mut tokio::sync::broadcast::Receiver<wisp_browser::cdp::CdpEvent>,
        sid: &str,
    ) -> Result<u16> {
        let t_nav = std::time::Instant::now();
        tracing::info!("BrowserWork[+CF]: {url} 导航");
        if let Err(e) = page.goto(url).await {
            tracing::warn!("BrowserWork[+CF]: {url} goto 失败: {e}");
            return Err(e);
        }
        tracing::trace!(elapsed_ms = t_nav.elapsed().as_millis(), url = %url, "goto timing");
        let t_status = std::time::Instant::now();
        let nav_status = match recv_navigation_status(event_rx, sid).await {
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
        Ok(nav_status)
    }

    pub(super) async fn solve_cf(
        &self,
        page: &mut Page,
        url: &str,
        nav_status: u16,
    ) -> Result<u16> {
        let t_cf = std::time::Instant::now();
        let solver = ChallengeSolver::new(page);
        solver
            .solve_with_config(self.challenge_timeout, &self.turnstile)
            .await?;
        tracing::trace!(elapsed_ms = t_cf.elapsed().as_millis(), url = %url, "solve_cf timing");
        if nav_status == 200 {
            return Ok(nav_status);
        }
        tracing::debug!("BrowserWork[+CF]: {url} CF 挑战解决，状态码 {nav_status} → 200");
        Ok(200)
    }

    pub(super) async fn simulate_human(&self, page: &mut Page) -> Result<()> {
        if !self.human_mode {
            return Ok(());
        }
        let human = HumanBehavior::new(page);
        human.random_delay(500, 1500).await?;
        human.random_scroll().await?;
        human.random_delay(300, 800).await
    }
}
