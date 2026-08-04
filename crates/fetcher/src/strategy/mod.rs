//! 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
//!
//! ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
//! `BrowserFetchStrategy` trait 已下沉到 wisp-browser（浏览器领域契约），
//! 本模块保留具体实现与 re-export。

mod extract;

#[cfg(feature = "browser")]
mod dynamic;
#[cfg(feature = "stealth")]
mod stealth;

#[cfg(feature = "browser")]
pub use dynamic::DynamicStrategy;
#[cfg(feature = "stealth")]
pub use stealth::StealthStrategy;

// 重导出浏览器领域契约，保持 `crate::strategy::BrowserFetchStrategy` 引用路径不变。
pub use wisp_browser::BrowserFetchStrategy;

#[cfg(test)]
mod tests;

pub(crate) use extract::extract_browser_response;
