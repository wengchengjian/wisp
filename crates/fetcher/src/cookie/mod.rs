//! 统一 Cookie 存储 trait — 跨 HTTP/浏览器/CF 三处 cookie 状态。
//!
//! ARCH: 解决 cookie 状态分散问题。FetchClient 持有 `Arc<dyn CookieJar>`，
//! strategy 可访问。三种实现：
//! - HttpCookieJar: 包装 wreq::cookie::Jar（与 wreq::Client 共享）
//! - BrowserCookieJar: 通过 CDP Network.getCookies/setCookie
//! - CfCookieJar: moka::Cache + 文件持久化（从 FetchClient 迁出）

#[cfg(feature = "browser")]
pub mod browser;
#[cfg(feature = "stealth")]
pub mod cf;
pub mod http;

#[cfg(feature = "browser")]
pub use browser::BrowserCookieJar;
#[cfg(feature = "stealth")]
pub use cf::{CfCookieJar, CfSession};
pub use http::HttpCookieJar;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use url::Url;

/// Cookie 表示（统一格式，跨 HTTP/浏览器/CF）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cookie {
    /// Cookie 名称。
    pub name: String,
    /// Cookie 值。
    pub value: String,
    /// Cookie 作用域名（如 "example.com"）。
    pub domain: String,
    /// Cookie 作用路径（如 "/"）。
    pub path: String,
    /// 是否仅 HTTPS 传输。
    pub secure: bool,
    /// 是否仅 HTTP 可访问（JS 不可读）。
    pub http_only: bool,
    /// SameSite 策略（"Strict"/"Lax"/"None"）。
    pub same_site: Option<String>,
    /// Unix 时间戳（秒），None 表示会话 cookie。
    pub expires: Option<f64>,
}

/// Cookie 存储 trait — 统一 HTTP/浏览器/CF 三处 cookie 状态。
///
/// ARCH: FetchClient 持有 `Arc<dyn CookieJar>`，strategy 可访问。
/// 三种实现见模块顶部文档。
#[async_trait]
pub trait CookieJar: Send + Sync {
    /// 获取指定 URL 的所有匹配 cookie（按 domain/path/secure 匹配）。
    async fn get(&self, url: &Url) -> Vec<Cookie>;

    /// 写入 cookie。
    async fn set(&self, cookie: Cookie);

    /// 删除指定 URL 匹配的所有 cookie（用于失效会话）。
    async fn clear(&self, url: &Url);

    /// 获取 Cookie 头字符串（用于 HTTP 请求注入）。
    /// 默认实现基于 `get()`，可被覆盖以优化。
    async fn header(&self, url: &Url) -> Option<String> {
        let cookies = self.get(url).await;
        if cookies.is_empty() {
            return None;
        }
        Some(
            cookies
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    /// 获取与 cookie 会话绑定的 User-Agent（如 CF 会话）。
    ///
    /// 默认返回 `None`；`CfCookieJar` 返回签发 cookie 时浏览器使用的实际 UA，
    /// 供 HTTP 快速路径保持 cookie 与 UA 一致性。
    async fn ua(&self, url: &Url) -> Option<String> {
        let _ = url;
        None
    }
}

/// 测试用 MockCookieJar — 内存实现，记录所有操作。
pub struct MockCookieJar {
    cookies: parking_lot::Mutex<Vec<Cookie>>,
}

/// 域名匹配：host 必须等于 domain 或以 `.domain` 结尾，避免后缀撞名。
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

impl MockCookieJar {
    /// 创建空 mock。
    #[must_use]
    pub fn new() -> Self {
        Self {
            cookies: parking_lot::Mutex::new(Vec::new()),
        }
    }
}

impl Default for MockCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CookieJar for MockCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let host = url.host_str().unwrap_or("");
        let path = url.path();
        self.cookies
            .lock()
            .iter()
            .filter(|c| {
                // 简化匹配：域名边界匹配 + path 前缀匹配
                domain_matches(host, &c.domain) && path.starts_with(&c.path)
            })
            .cloned()
            .collect()
    }

    async fn set(&self, cookie: Cookie) {
        let mut guard = self.cookies.lock();
        // 替换同名同 domain 同 path 的 cookie
        guard.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        guard.push(cookie);
    }

    async fn clear(&self, url: &Url) {
        let host = url.host_str().unwrap_or("");
        let mut guard = self.cookies.lock();
        guard.retain(|c| !domain_matches(host, &c.domain));
    }
}

#[cfg(test)]
mod tests;
