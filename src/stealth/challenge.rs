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

/// Check if a response/page comes from Cloudflare (by checking headers or page content).
pub async fn is_cloudflare_page(page: &Page) -> Result<bool> {
    let js = r#"(() => {
        // Check for CF-specific elements or headers
        const body = document.body ? document.body.innerHTML : '';
        return body.includes('cloudflare') ||
               body.includes('cf-browser-verification') ||
               body.includes('challenge-platform') ||
               document.title.includes('Just a moment') ||
               !!document.querySelector('[class*="cf-"]');
    })()"#;

    let result = page.evaluate(js).await?;
    Ok(result.as_bool().unwrap_or(false))
}
