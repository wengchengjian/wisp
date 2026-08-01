//! 挑战自动解决。

use std::time::Duration;

use crate::turnstile;
use crate::TurnstileConfig;

use super::{ChallengeSolver, ChallengeType};
use wisp_browser::page::Page;
use wisp_core::error::{Result, WispError};

impl<'a> ChallengeSolver<'a> {
    /// 创建挑战解决器。
    pub fn new(page: &'a Page) -> Self {
        Self { page }
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
            match challenge {
                ChallengeType::None => return Ok(()),
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
