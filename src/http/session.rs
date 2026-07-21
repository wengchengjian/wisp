//! HTTP Session with automatic cookie persistence.
//!
//! 类似 Scrapling 的 FetcherSession：跨请求保持 cookies 和状态。
//!
//! # 示例
//!
//! ```rust,no_run
//! use wisp::http::HttpSession;
//!
//! # async fn example() -> wisp::Result<()> {
//! let session = HttpSession::new(Default::default())?;
//!
//! // 第一次请求：服务器设置 cookie
//! let resp1 = session.get("https://httpbin.org/cookies/set?token=abc123").await?;
//!
//! // 第二次请求：自动携带之前的 cookie
//! let resp2 = session.get("https://httpbin.org/cookies").await?;
//! let json = resp2.json()?;
//! assert_eq!(json["cookies"]["token"], "abc123");
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use serde_json::Value;

use crate::error::Result;
use super::{Client, Config, Response};

/// 带 cookie 持久化的 HTTP 会话。
///
/// 所有请求自动携带之前响应中设置的 cookies，
/// 并自动从 Set-Cookie 响应头更新 cookie 存储。
#[derive(Clone)]
pub struct HttpSession {
    client: Client,
    /// cookie 存储：domain -> (name -> value)
    cookies: Arc<Mutex<HashMap<String, HashMap<String, String>>>>,
}

impl HttpSession {
    /// 创建新会话。
    pub fn new(config: Config) -> Result<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()?;
        Ok(Self {
            client,
            cookies: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// 从已有 Client 创建会话。
    pub fn from_client(client: Client) -> Self {
        Self {
            client,
            cookies: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 使用 builder 风格创建会话。
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// GET 请求（自动携带/更新 cookies）。
    pub async fn get(&self, url: &str) -> Result<Response> {
        let cookie_header = self.cookie_header_for(url).await;
        let resp = self.client.get_with_headers(url, &cookie_header).await?;
        self.store_cookies_from_response(url, &resp).await;
        Ok(resp)
    }

    /// POST 请求（自动携带/更新 cookies）。
    pub async fn post(&self, url: &str, body: Option<&str>, json: Option<&Value>) -> Result<Response> {
        let cookie_header = self.cookie_header_for(url).await;
        let resp = self.client.post_with_headers(url, body, json, &cookie_header).await?;
        self.store_cookies_from_response(url, &resp).await;
        Ok(resp)
    }

    /// 手动设置 cookie。
    pub async fn set_cookie(&self, domain: &str, name: &str, value: &str) {
        let mut cookies = self.cookies.lock().await;
        cookies
            .entry(domain.to_string())
            .or_default()
            .insert(name.to_string(), value.to_string());
    }

    /// 获取指定域名的所有 cookies。
    pub async fn cookies_for(&self, domain: &str) -> HashMap<String, String> {
        let cookies = self.cookies.lock().await;
        cookies.get(domain).cloned().unwrap_or_default()
    }

    /// 清除所有 cookies。
    pub async fn clear_cookies(&self) {
        self.cookies.lock().await.clear();
    }

    /// 获取内部 Client 引用。
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// 构造适用于给定 URL 的 Cookie header 值。
    async fn cookie_header_for(&self, url: &str) -> Vec<(String, String)> {
        let domain = extract_domain(url);
        let cookies = self.cookies.lock().await;

        let mut headers = Vec::new();
        // 匹配所有可能的域名（精确匹配 + 父域匹配）
        for (stored_domain, pairs) in cookies.iter() {
            if domain.ends_with(stored_domain.as_str()) {
                let cookie_str: String = pairs
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join("; ");
                if !cookie_str.is_empty() {
                    headers.push(("Cookie".to_string(), cookie_str));
                }
            }
        }
        headers
    }

    /// 从响应的 Set-Cookie 头提取并存储 cookies。
    async fn store_cookies_from_response(&self, url: &str, resp: &Response) {
        let domain = extract_domain(url);
        let mut cookies = self.cookies.lock().await;

        // 检查所有 set-cookie 头
        for (key, value) in &resp.headers {
            if key.eq_ignore_ascii_case("set-cookie") {
                // 解析 "name=value; Path=/; Domain=.example.com" 格式
                if let Some(name_value) = value.split(';').next() {
                    if let Some((name, val)) = name_value.split_once('=') {
                        let name = name.trim().to_string();
                        let val = val.trim().to_string();
                        if !name.is_empty() {
                            cookies
                                .entry(domain.clone())
                                .or_default()
                                .insert(name, val);
                        }
                    }
                }
            }
        }
    }
}

/// Session 构建器。
pub struct SessionBuilder {
    config: Config,
}

impl SessionBuilder {
    pub fn new() -> Self {
        Self { config: Config::default() }
    }

    pub fn timeout(mut self, d: std::time::Duration) -> Self {
        self.config.timeout = d;
        self
    }

    pub fn proxy(mut self, url: &str) -> Self {
        self.config.proxy = Some(url.to_string());
        self
    }

    pub fn user_agent(mut self, ua: &str) -> Self {
        self.config.user_agent = Some(ua.to_string());
        self
    }

    pub fn build(self) -> Result<HttpSession> {
        HttpSession::new(self.config)
    }
}

/// 从 URL 提取域名。
fn extract_domain(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain() {
        assert_eq!(extract_domain("https://example.com/path"), "example.com");
        assert_eq!(extract_domain("http://sub.domain.org:8080/x"), "sub.domain.org");
        assert_eq!(extract_domain("invalid"), "");
    }

    #[tokio::test]
    async fn test_session_set_and_get_cookies() {
        let session = HttpSession::new(Config::default()).unwrap();
        session.set_cookie("example.com", "token", "abc123").await;
        session.set_cookie("example.com", "sid", "xyz").await;

        let cookies = session.cookies_for("example.com").await;
        assert_eq!(cookies.get("token").unwrap(), "abc123");
        assert_eq!(cookies.get("sid").unwrap(), "xyz");
    }

    #[tokio::test]
    async fn test_session_clear_cookies() {
        let session = HttpSession::new(Config::default()).unwrap();
        session.set_cookie("example.com", "a", "1").await;
        session.clear_cookies().await;
        let cookies = session.cookies_for("example.com").await;
        assert!(cookies.is_empty());
    }

    #[tokio::test]
    async fn test_cookie_header_for() {
        let session = HttpSession::new(Config::default()).unwrap();
        session.set_cookie("example.com", "token", "abc").await;

        let headers = session.cookie_header_for("https://example.com/page").await;
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "Cookie");
        assert!(headers[0].1.contains("token=abc"));
    }

    #[tokio::test]
    async fn test_cookie_domain_matching() {
        let session = HttpSession::new(Config::default()).unwrap();
        session.set_cookie("example.com", "global", "yes").await;

        // 子域名应匹配父域 cookie
        let headers = session.cookie_header_for("https://sub.example.com/page").await;
        assert_eq!(headers.len(), 1);
        assert!(headers[0].1.contains("global=yes"));

        // 不相关域名不匹配
        let headers = session.cookie_header_for("https://other.org/").await;
        assert!(headers.is_empty());
    }

    #[test]
    fn test_session_builder() {
        let session = HttpSession::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent("test-agent")
            .build();
        assert!(session.is_ok());
    }
}
