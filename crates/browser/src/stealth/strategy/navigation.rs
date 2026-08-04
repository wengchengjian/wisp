//! CF 挑战解决与人类行为模拟。

use super::*;

use crate::stealth::challenge::ChallengeSolver;
use crate::stealth::human::HumanBehavior;

impl StealthStrategy {
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
        tracing::debug!("BrowserWork[+CF]: {url} CF 挑战解决，状态码 {nav_status} -> 200");
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
