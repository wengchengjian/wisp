//! Cloudflare challenge detection and automatic solving.
//!
//! Supports: JS Challenge (5-second shield), Turnstile, Managed Challenge.

mod detect;
mod solve;

#[cfg(test)]
mod tests;

use wisp_browser::page::Page;

pub use detect::is_cloudflare_page;

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
