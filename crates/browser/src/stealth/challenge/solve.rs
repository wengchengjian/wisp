//! 挑战自动解决。

use std::time::Duration;

use crate::page::Page;
use crate::stealth::turnstile;
use wisp_core::error::{Result, WispError};
use wisp_core::stealth::TurnstileConfig;

use super::{ChallengeSolver, ChallengeType};

impl<'a> ChallengeSolver<'a> {
    /// 创建挑战解决器。
    pub fn new(page: &'a Page) -> Self {
        Self { page }
    }

    /// 轻量判断页面是否仍处于 CF 挑战占位页（"请稍候"/"Just a moment" 等）。
    ///
    /// 仅读取 document.title，不走完整 DOM 遍历，降低被 CF 反爬识别的风险。
    async fn is_pending_challenge(&self) -> Result<bool> {
        let js = r#"(() => {
            const t = document.title || '';
            return t.includes('请稍候') || t.includes('请稍後') ||
                   t.includes('Just a moment') || t.includes('Attention Required') ||
                   t.includes('正在进行安全验证');
        })()"#;
        let result = self.page.evaluate(js).await?;
        Ok(result.as_bool().unwrap_or(false))
    }

    /// Detect and automatically solve any Cloudflare challenge.
    /// Loops: re-detects challenge type and handles transitions (e.g., JS shield -> Turnstile).
    pub async fn solve(&self, timeout: Duration) -> Result<()> {
        self.solve_with_config(timeout, &TurnstileConfig::default())
            .await
    }

    /// 使用自定义 Turnstile 配置解决挑战。
    pub async fn solve_with_config(
        &self,
        timeout: Duration,
        turnstile_cfg: &TurnstileConfig,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout(
                    "Cloudflare challenge did not resolve in time".into(),
                ));
            }

            let challenge = self.detect().await?;
            tracing::info!("solve_cf: challenge={challenge:?}");
            match challenge {
                ChallengeType::None => {
                    // 无已知挑战标记，但页面仍处于"请稍候"占位页时，说明挑战进行中
                    //（turnstile 尚未渲染 / JS 挑战自动跳转中）。等待其完成后重新检测，
                    // 避免过早提取挑战占位页。仅轻量读 title，降低被 CF 识别的风险。
                    if self.is_pending_challenge().await? {
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                    } else {
                        return Ok(());
                    }
                }
                ChallengeType::JsChallenge => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                ChallengeType::Turnstile => {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    return turnstile::solve_turnstile_with_config(
                        self.page,
                        remaining,
                        turnstile_cfg,
                    )
                    .await;
                }
                ChallengeType::ManagedChallenge => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }
}
