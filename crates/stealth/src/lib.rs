//! Stealth module: Cloudflare challenge solving + human behavior simulation.
//!
//! Unified interface for anti-detection capabilities.

pub mod challenge;
pub mod human;
pub mod turnstile;

pub use challenge::{is_cloudflare_page, ChallengeSolver, ChallengeType};
pub use human::HumanBehavior;
pub use turnstile::TurnstileConfig;
