//! 挑战检测。

use super::{ChallengeSolver, ChallengeType};
use crate::page::Page;
use wisp_core::error::Result;

const DETECTION_JS: &str = r#"(() => {
    const title = document.title || '';
    const body = document.body ? document.body.innerHTML : '';
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
    if (document.querySelector('.cf-turnstile') ||
        document.querySelector('iframe[src*="challenges.cloudflare.com"]') ||
        document.querySelector('iframe[id*="cf-chl"]') ||
        body.includes('cf-turnstile') ||
        body.includes('cf-chl-widget') ||
        findInShadows()) {
        return 'turnstile';
    }
    const raw = document.documentElement ? document.documentElement.outerHTML : '';
    if (raw.includes('_cf_chl_opt') && /cType\s*:\s*['"]interactive['"]/.test(raw)) {
        return 'turnstile';
    }
    if (title.includes('Just a moment') ||
        title.includes('Attention Required') ||
        document.querySelector('#challenge-running') ||
        document.querySelector('.cf-browser-verification') ||
        document.querySelector('#cf-challenge-running') ||
        body.includes('cf-challenge-running')) {
        return 'js_challenge';
    }
    if (document.querySelector('#challenge-stage') ||
        body.includes('managed_checking_msg')) {
        return 'managed';
    }
    return 'none';
})()"#;

fn classify_challenge(challenge: &str) -> ChallengeType {
    match challenge {
        "turnstile" => ChallengeType::Turnstile,
        "js_challenge" => ChallengeType::JsChallenge,
        "managed" => ChallengeType::ManagedChallenge,
        _ => ChallengeType::None,
    }
}

impl ChallengeSolver<'_> {
    /// Detect what type of Cloudflare challenge is on the current page.
    pub async fn detect(&self) -> Result<ChallengeType> {
        let result = self.page.evaluate(DETECTION_JS).await?;
        Ok(classify_challenge(result.as_str().unwrap_or("none")))
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
