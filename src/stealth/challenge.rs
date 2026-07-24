//! Cloudflare challenge detection and automatic solving.
//!
//! Supports: JS Challenge (5-second shield), Turnstile, Managed Challenge.
use super::turnstile;


use std::time::Duration;

use crate::error::{WispError, Result};
use crate::browser::page::Page;

/// Type of Cloudflare challenge detected on the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChallengeType {
    /// No challenge detected.
    None,
    /// JavaScript challenge (5-second shield / IUAM).
    JsChallenge,
    /// Cloudflare Turnstile widget.
    Turnstile,
    /// Managed challenge (Cloudflare decides which to show).
    ManagedChallenge,
}

/// Detects and solves Cloudflare challenges using a real browser.
pub struct ChallengeSolver<'a> {
    page: &'a Page,
}

impl<'a> ChallengeSolver<'a> {
    pub fn new(page: &'a Page) -> Self {
        Self { page }
    }

    /// Detect what type of Cloudflare challenge is on the current page.
    pub async fn detect(&self) -> Result<ChallengeType> {
        let detection_js = r#"(() => {
            const title = document.title || '';
            const body = document.body ? document.body.innerHTML : '';

            // Helper: search shadow roots for Turnstile iframe
            function findInShadows() {
                const els = document.querySelectorAll('*');
                for (const el of els) {
                    if (el.shadowRoot) {
                        if (el.shadowRoot.querySelector('iframe[src*="challenges.cloudflare.com"]') ||
                            el.shadowRoot.querySelector('iframe[id*="cf-chl"]')) return true;
                    }
                }
                return false;
            }

            // Turnstile widget (direct + shadow roots)
            if (document.querySelector('.cf-turnstile') ||
                document.querySelector('iframe[src*="challenges.cloudflare.com"]') ||
                document.querySelector('iframe[id*="cf-chl"]') ||
                body.includes('cf-turnstile') ||
                body.includes('cf-chl-widget') ||
                findInShadows()) {
                return 'turnstile';
            }

            // JS Challenge (5-second shield)
            if (title.includes('Just a moment') ||
                title.includes('Attention Required') ||
                document.querySelector('#challenge-running') ||
                document.querySelector('.cf-browser-verification') ||
                document.querySelector('#cf-challenge-running') ||
                body.includes('cf-challenge-running')) {
                return 'js_challenge';
            }

            // Managed challenge
            if (document.querySelector('#challenge-stage') ||
                body.includes('challenge-platform') ||
                body.includes('managed_checking_msg')) {
                return 'managed';
            }

            return 'none';
        })()"#;

        let result = self.page.evaluate(detection_js).await?;
        let challenge_str = result.as_str().unwrap_or("none");

        Ok(match challenge_str {
            "turnstile" => ChallengeType::Turnstile,
            "js_challenge" => ChallengeType::JsChallenge,
            "managed" => ChallengeType::ManagedChallenge,
            _ => ChallengeType::None,
        })
    }

    /// Detect and automatically solve any Cloudflare challenge.
    /// Loops: re-detects challenge type and handles transitions (e.g., JS shield -> Turnstile).
    pub async fn solve(&self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(WispError::Timeout("Cloudflare challenge did not resolve in time".into()));
            }

            let challenge = self.detect().await?;
            match challenge {
                ChallengeType::None => return Ok(()),
                ChallengeType::JsChallenge => {
                    // JS challenge: wait a bit, it may auto-solve or transition to Turnstile
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                ChallengeType::Turnstile => {
                    // Turnstile: use CDP pierce + click solver
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    return turnstile::solve_turnstile(self.page, remaining).await;
                }
                ChallengeType::ManagedChallenge => {
                    // Managed: wait, may transition to Turnstile
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }
}

/// Check if a response/page comes from Cloudflare (by checking challenge-specific markers).
///
/// 仅检测 CF 挑战页特有标识，避免对普通提及 Cloudflare 的页面误判。
pub async fn is_cloudflare_page(page: &Page) -> Result<bool> {
    let js = r#"(() => {
        const body = document.body ? document.body.innerHTML : '';
        const title = document.title || '';
        return body.includes('cf-browser-verification') ||
               body.includes('challenge-platform') ||
               body.includes('cf-challenge-running') ||
               body.includes('cf-chl-bypass') ||
               title.includes('Just a moment') ||
               title.includes('Attention Required') ||
               !!document.querySelector('#challenge-running') ||
               !!document.querySelector('.cf-turnstile');
    })()"#;

    let result = page.evaluate(js).await?;
    Ok(result.as_bool().unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    // === ChallengeType 枚举单元测试 ===

    #[test]
    fn test_challenge_type_equality() {
        assert_eq!(ChallengeType::None, ChallengeType::None);
        assert_eq!(ChallengeType::JsChallenge, ChallengeType::JsChallenge);
        assert_eq!(ChallengeType::Turnstile, ChallengeType::Turnstile);
        assert_eq!(ChallengeType::ManagedChallenge, ChallengeType::ManagedChallenge);
        assert_ne!(ChallengeType::None, ChallengeType::JsChallenge);
        assert_ne!(ChallengeType::Turnstile, ChallengeType::ManagedChallenge);
    }

    #[test]
    fn test_challenge_type_clone_copy() {
        let ct = ChallengeType::Turnstile;
        let cloned = ct.clone();
        let copied = ct; // Copy
        assert_eq!(ct, cloned);
        assert_eq!(ct, copied);
    }

    #[test]
    fn test_challenge_type_debug() {
        assert_eq!(format!("{:?}", ChallengeType::None), "None");
        assert_eq!(format!("{:?}", ChallengeType::JsChallenge), "JsChallenge");
        assert_eq!(format!("{:?}", ChallengeType::Turnstile), "Turnstile");
        assert_eq!(format!("{:?}", ChallengeType::ManagedChallenge), "ManagedChallenge");
    }

    // === 集成测试（需要 Chrome 环境） ===

    /// 测试 ChallengeSolver::detect() 在普通页面上返回 None。
    /// 需要本地 Chrome/Chromium 环境。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_detect_no_challenge_on_normal_page() {
        use crate::browser::Browser;
        use crate::config::LaunchOptions;

        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");
        // data: URL 不触发任何 CF 挑战
        page.goto("data:text/html,<html><body><h1>Hello</h1></body></html>")
            .await
            .expect("导航");

        let solver = ChallengeSolver::new(&page);
        let result = solver.detect().await.expect("detect 应成功");
        assert_eq!(result, ChallengeType::None, "普通页面不应检测到挑战");

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }

    /// 测试 is_cloudflare_page 在普通页面上返回 false。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_is_cloudflare_page_normal() {
        use crate::browser::Browser;
        use crate::config::LaunchOptions;

        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");
        page.goto("data:text/html,<html><body><p>Normal content</p></body></html>")
            .await
            .expect("导航");

        let result = is_cloudflare_page(&page).await.expect("检测应成功");
        assert!(!result, "普通页面不应被误判为 CF 挑战页");

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }

    /// 测试包含 CF 特有标识的页面被正确检测。
    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn test_detect_js_challenge_markers() {
        use crate::browser::Browser;
        use crate::config::LaunchOptions;

        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");
        // 模拟 JS Challenge 页面（包含特有标识）
        page.goto("data:text/html,<html><head><title>Just a moment...</title></head><body><div id='challenge-running'>Checking...</div></body></html>")
            .await
            .expect("导航");

        let solver = ChallengeSolver::new(&page);
        let result = solver.detect().await.expect("detect 应成功");
        assert_eq!(result, ChallengeType::JsChallenge, "应检测到 JS Challenge");

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }
}
