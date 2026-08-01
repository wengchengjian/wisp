//! 内建中间件实现。

use super::{CrawlContext, ErrorAction, Middleware, RequestMwAction, ResponseMwAction};

// === 子模块 ===
mod cache_robots_delay;
mod challenge;
mod defaults;
mod request;
mod retry;
mod upgrade;

pub use cache_robots_delay::{CacheMiddleware, DelayMiddleware, RobotsMiddleware};
pub use challenge::CookieChallengeMiddleware;
pub use defaults::{default_middlewares, DefaultMiddlewareConfig};
pub use request::{
    HeadersMiddleware, ProxyInjectionMiddleware, RetryMiddleware, UaRotationMiddleware,
};
pub use retry::BlockedRetryMiddleware;
pub use upgrade::{DynamicUpgradeMiddleware, StealthUpgradeMiddleware};
#[cfg(test)]
mod tests;
