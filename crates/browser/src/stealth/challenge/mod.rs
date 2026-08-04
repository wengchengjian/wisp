//! Cloudflare challenge detection and automatic solving.
//!
//! Supports: JS Challenge (5-second shield), Turnstile, Managed Challenge.

mod detect;
mod solve;

#[cfg(test)]
mod tests;

use crate::page::Page;

pub use detect::is_cloudflare_page;
pub use wisp_core::stealth::ChallengeType;

/// Detects and solves Cloudflare challenges using a real browser.
pub struct ChallengeSolver<'a> {
    page: &'a Page,
}