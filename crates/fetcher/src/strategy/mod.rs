//! 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
//!
//! ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
//! 新策略（如 Playwright）可实现此 trait 零侵入注入。

mod extract;

#[cfg(feature = "browser")]
mod dynamic;
#[cfg(feature = "stealth")]
mod stealth;

#[cfg(feature = "browser")]
pub use dynamic::DynamicStrategy;
#[cfg(feature = "stealth")]
pub use stealth::StealthStrategy;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use wisp_browser::Page;
use wisp_core::error::Result;
use wisp_core::{Request, Response};

/// 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
///
/// ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
/// 新策略（如 Playwright）可实现此 trait 零侵入注入。
///
/// 调用方（`FetchClient::fetch_browser`）保证：
/// - 调用前已 `acquire` page
/// - 调用后由调用方 `close` page
/// - 120s 总超时由调用方包装
#[async_trait]
pub trait BrowserFetchStrategy: Send + Sync {
    /// 执行浏览器导航 + 后处理（CF 挑战 / 人类行为 / 等待选择器等）。
    async fn fetch(&self, page: &mut Page, req: &Request) -> Result<Response>;
}

pub(crate) use extract::extract_browser_response;
