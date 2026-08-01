//! Cloudflare Turnstile challenge solving via CDP shadow DOM piercing.
//!
//! Key technique: Turnstile renders inside a closed shadow DOM.
//! Normal JS cannot access it. We use CDP DOM.getDocument(pierce=true)
//! to find the iframe node, then DOM.getContentQuads for coordinates.

mod check;
mod click;
mod config;
mod solve;

pub use config::TurnstileConfig;
pub use solve::{solve_turnstile, solve_turnstile_with_config};
