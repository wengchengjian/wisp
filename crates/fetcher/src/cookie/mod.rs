//! 统一 Cookie 存储 — 各后端实现 + 聚合。
//!
//! `Cookie` / `CookieJar` trait 已下沉到 wisp-core（领域契约），
//! `BrowserCookieJar` 已下沉到 wisp-browser（浏览器领域）。
//! 本模块只保留 HTTP/CF 后端与聚合：HttpCookieJar / CfCookieJar / CompositeCookieJar。

#[cfg(feature = "stealth")]
pub mod cf;
mod composite;
pub mod http;

#[cfg(feature = "stealth")]
pub use cf::{CfCookieJar, CfSession};
pub use composite::CompositeCookieJar;
pub use http::HttpCookieJar;

// 重导出核心类型，保持 `crate::cookie::{Cookie, CookieJar}` 引用路径不变。
pub use wisp_core::cookie::{Cookie, CookieJar, MockCookieJar};
// 从 wisp-browser 重导出，保持 `crate::cookie::BrowserCookieJar` 引用路径不变。
#[cfg(feature = "browser")]
pub use wisp_browser::BrowserCookieJar;
