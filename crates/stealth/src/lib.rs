//! Stealth module: Cloudflare challenge solving + human behavior simulation.
//!
//! 薄壳：反检测引擎已迁入 `wisp-browser::stealth`，此处仅重新导出以保持 API 兼容。

pub use wisp_browser::stealth::{challenge, human, turnstile};
pub use wisp_browser::stealth::{
    ChallengeSolver, ChallengeType, HumanBehavior, StealthConfig, StealthStrategy, TurnstileConfig,
    is_cloudflare_page,
};