//! Turnstile 解决主循环。

use std::time::Duration;

use super::check::check_bypassed;
use super::click::try_click_turnstile_cdp;
use crate::page::Page;
use rand::rngs::{SmallRng, SysRng};
use rand::{RngExt, SeedableRng};
use wisp_core::error::{Result, WispError};
use wisp_core::stealth::TurnstileConfig;

/// Solve a Cloudflare Turnstile challenge on the given page.
pub async fn solve_turnstile(page: &Page, timeout: Duration) -> Result<()> {
    solve_turnstile_with_config(page, timeout, &TurnstileConfig::default()).await
}

fn turnstile_timeout_error(elapsed: Duration, click_count: u32) -> WispError {
    WispError::Timeout(format!(
        "Turnstile not solved after {:.0}s ({} clicks)",
        elapsed.as_secs_f64(),
        click_count
    ))
}

async fn check_bypassed_with_retry(page: &Page, start: tokio::time::Instant) -> Result<bool> {
    let elapsed = start.elapsed();
    match check_bypassed(page).await {
        Ok(true) => Ok(true),
        Ok(false) => Ok(false),
        Err(e) => {
            println!(
                "[turnstile] {:.1}s: check error: {}",
                elapsed.as_secs_f64(),
                e
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
            if check_bypassed(page).await.unwrap_or(false) {
                println!(
                    "[turnstile] {:.1}s: bypassed after retry",
                    start.elapsed().as_secs_f64()
                );
                return Ok(true);
            }
            Ok(false)
        }
    }
}

async fn maybe_click_turnstile(
    page: &Page,
    cfg: &TurnstileConfig,
    click_count: u32,
    elapsed: Duration,
    last_click: tokio::time::Instant,
) -> (u32, tokio::time::Instant) {
    let passive_wait = Duration::from_millis(cfg.passive_wait_ms);
    // human_mode 下点击间隔随机化（基准 click_interval 上叠加随机抖动），
    // 避免固定周期点击被视作自动化行为。
    let base = Duration::from_millis(cfg.click_interval_ms);
    let click_interval = if cfg.human_mode {
        let mut rng = SmallRng::try_from_rng(&mut SysRng).expect("OS RNG failed");
        base + Duration::from_millis(rng.random_range(0..=900))
    } else {
        base
    };
    if elapsed <= passive_wait || last_click.elapsed() < click_interval {
        return (click_count, last_click);
    }
    let click_count = click_count + 1;
    let t1 = tokio::time::Instant::now();
    let clicked = try_click_turnstile_cdp(page, click_count, cfg).await;
    println!(
        "[turnstile] {:.1}s: click #{} {} ({:.0}ms)",
        elapsed.as_secs_f64(),
        click_count,
        if clicked { "OK" } else { "iframe not found" },
        t1.elapsed().as_millis()
    );
    (click_count, tokio::time::Instant::now())
}

/// 使用自定义配置解决 Turnstile 挑战。
pub async fn solve_turnstile_with_config(
    page: &Page,
    timeout: Duration,
    cfg: &TurnstileConfig,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut click_count: u32 = 0;
    let mut last_click = tokio::time::Instant::now();
    let start = tokio::time::Instant::now();

    loop {
        let elapsed = start.elapsed();
        if tokio::time::Instant::now() > deadline {
            return Err(turnstile_timeout_error(elapsed, click_count));
        }
        if check_bypassed_with_retry(page, start).await? {
            println!(
                "[turnstile] {:.1}s: bypassed detected",
                elapsed.as_secs_f64()
            );
            return Ok(());
        }
        let (new_count, new_last) =
            maybe_click_turnstile(page, cfg, click_count, elapsed, last_click).await;
        click_count = new_count;
        last_click = new_last;
        tokio::time::sleep(Duration::from_millis(cfg.poll_interval_ms)).await;
    }
}
