//! 反检测引擎：Cloudflare 挑战解决 + 人类行为模拟 + Stealth 抓取策略。
//!
//! 从 wisp-stealth / fetcher 迁入 browser 领域，使浏览器自洽，不再依赖高层
//! 反向依赖。`TurnstileConfig` / `ChallengeType` 为纯配置类型，下沉到 core。

pub mod challenge;
pub mod human;
pub mod strategy;
pub mod turnstile;

pub use challenge::{ChallengeSolver, ChallengeType, is_cloudflare_page};
pub use human::HumanBehavior;
pub use strategy::{StealthConfig, StealthStrategy};
pub use turnstile::TurnstileConfig;