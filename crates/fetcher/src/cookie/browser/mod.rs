//! 浏览器 cookie jar — 通过 CDP Network.getCookies/setCookie/clearBrowserCookies。
//!
//! ARCH: 每个 Page 持有一个 BrowserCookieJar，导航后可读取 cookie，
//! ChallengeSolver 解决 CF 后将 cookie 写入此 jar。

mod jar;

#[cfg(test)]
mod tests;

pub use jar::BrowserCookieJar;
