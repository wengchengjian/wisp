//! 统一 Cookie 存储 trait — 跨 HTTP/浏览器/CF 三处 cookie 状态。
//!
//! ARCH: 解决 cookie 状态分散问题。FetchClient 持有 `Arc<dyn CookieJar>`，
//! strategy 可访问。三种实现：
//! - HttpCookieJar: 包装 wreq::cookie::Jar（与 wreq::Client 共享）
//! - BrowserCookieJar: 通过 CDP Network.getCookies/setCookie
//! - CfCookieJar: moka::Cache + 文件持久化（从 FetchClient 迁出）

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use url::Url;

pub mod cf;
pub mod http;

pub use cf::{CfCookieJar, CfSession};
pub use http::HttpCookieJar;

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
}

/// 测试用 MockCookieJar — 内存实现，记录所有操作。
pub struct MockCookieJar {
    cookies: parking_lot::Mutex<Vec<Cookie>>,
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
                // 简化匹配：domain 后缀匹配 + path 前缀匹配
                host.ends_with(&c.domain) && path.starts_with(&c.path)
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
        guard.retain(|c| !host.ends_with(&c.domain));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn make_url(s: &str) -> Url {
        Url::parse(s).expect("合法 URL")
    }

    fn make_cookie(name: &str, value: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: Some("Lax".into()),
            expires: None,
        }
    }

    #[tokio::test]
    async fn mock_set_and_get_cookie() {
        let jar = MockCookieJar::new();
        let url = make_url("https://example.com/path");
        jar.set(make_cookie("session", "abc123", "example.com"))
            .await;

        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "session");
        assert_eq!(cookies[0].value, "abc123");
    }

    #[tokio::test]
    async fn mock_set_replaces_same_name() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("session", "v1", "example.com")).await;
        jar.set(make_cookie("session", "v2", "example.com")).await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1, "同名 cookie 应被替换");
        assert_eq!(cookies[0].value, "v2");
    }

    #[tokio::test]
    async fn mock_clear_removes_matching_domain() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("a", "1", "example.com")).await;
        jar.set(make_cookie("b", "2", "other.com")).await;

        let url = make_url("https://example.com/");
        jar.clear(&url).await;

        let cookies = jar.get(&url).await;
        assert!(cookies.is_empty(), "example.com 的 cookie 应被清除");
    }

    #[tokio::test]
    async fn mock_header_returns_joined_string() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("a", "1", "example.com")).await;
        jar.set(make_cookie("b", "2", "example.com")).await;

        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert!(header.is_some());
        let header = header.expect("非空");
        assert!(header.contains("a=1"));
        assert!(header.contains("b=2"));
        assert!(header.contains("; "));
    }

    #[tokio::test]
    async fn mock_header_none_when_empty() {
        let jar = MockCookieJar::new();
        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert!(header.is_none());
    }

    #[tokio::test]
    async fn mock_domain_filter_excludes_other_domains() {
        let jar = MockCookieJar::new();
        jar.set(make_cookie("a", "1", "example.com")).await;
        jar.set(make_cookie("b", "2", "other.com")).await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "a");
    }

    #[test]
    fn cookie_serialization_roundtrip() {
        let c = make_cookie("test", "val", "example.com");
        let json = serde_json::to_string(&c).expect("序列化");
        let deserialized: Cookie = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(c, deserialized);
    }
}
