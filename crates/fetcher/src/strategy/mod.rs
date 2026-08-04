//! 浏览器抓取策略 — 区分 Dynamic/Stealth 的差异化逻辑。
//!
//! ARCH: 替代 `fetch_browser(&req, solve_cf: bool)` 的 bool 标志。
//! `BrowserFetchStrategy` trait 已下沉到 wisp-browser（浏览器领域契约），
//! `StealthStrategy` 亦已迁入 wisp-browser（完整反检测引擎），本模块仅保留
//! `DynamicStrategy` 具体实现，并 re-export 浏览器领域契约。

#[cfg(feature = "browser")]
mod dynamic;

#[cfg(feature = "browser")]
pub use dynamic::DynamicStrategy;
// StealthStrategy 已迁入 browser，此处 re-export 保持 `crate::strategy::StealthStrategy` 路径不变。
#[cfg(feature = "stealth")]
pub use wisp_browser::stealth::StealthStrategy;

// 重导出浏览器领域契约，保持 `crate::strategy::BrowserFetchStrategy` 引用路径不变。
pub use wisp_browser::BrowserFetchStrategy;

#[cfg(test)]
mod tests;
