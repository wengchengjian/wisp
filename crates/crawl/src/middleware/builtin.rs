//! 内建中间件实现。

use super::{CrawlContext, ErrorAction, Middleware, RequestMwAction, ResponseMwAction};

// === 子模块 ===
mod challenge;
mod defaults;
mod limit;
mod request;
mod retry;
mod upgrade;

pub use challenge::CookieChallengeMiddleware;
pub use defaults::{default_middlewares, DefaultMiddlewareConfig};
pub use limit::{CacheMiddleware, DelayMiddleware, RobotsMiddleware};
pub use request::{
    HeadersMiddleware, ProxyInjectionMiddleware, RetryMiddleware, UaRotationMiddleware,
};
pub use retry::BlockedRetryMiddleware;
pub use upgrade::{DynamicUpgradeMiddleware, StealthUpgradeMiddleware};
#[cfg(test)]
mod tests;
