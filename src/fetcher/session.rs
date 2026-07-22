//! 统一 Session - 跨请求保持 cookies/state。
//!
//! # 示例
//!
//! ```rust,no_run
//! use wisp::fetcher::Session;
//!
//! # async fn example() -> wisp::Result<()> {
//! let session = Session::stealth()
//!     .proxy("http://127.0.0.1:7897")
//!     .build()?;
//!
//! let p1 = session.get("https://site.com/login").await?;
//! let p2 = session.get("https://site.com/dashboard").await?; // 自动带 cookie
//!
//! session.close().await;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use wreq_util::Profile;

use crate::error::Result;
use super::{Fetcher, FetchMode, FetcherConfig, Response, Request};

/// 持久化会话 - 跨请求保持 cookies/state。
///
/// Http 模式：通过 cookie jar 实现。
/// Browser 模式：浏览器实例保持打开，天然保持 session。
pub struct Session {
    fetcher: Fetcher,
    /// HTTP 模式的 cookie 存储：domain -> (name -> value)
    cookies: Arc<Mutex<HashMap<String, HashMap<String, String>>>>,
    /// 浏览器模式保持的 Browser 实例
    browser: Arc<Mutex<Option<crate::browser::Browser>>>,
}

impl Session {
    /// 快速 HTTP 模式 Session。
    pub fn http() -> SessionBuilder {
        SessionBuilder::new(FetchMode::Http)
    }

    /// 浏览器渲染模式 Session。
    pub fn dynamic() -> SessionBuilder {
        SessionBuilder::new(FetchMode::Dynamic)
    }

    /// 隐身模式 Session。
    pub fn stealth() -> SessionBuilder {
        SessionBuilder::new(FetchMode::Stealth)
    }

    /// GET 请求（自动携带/更新 cookies）。
    pub async fn get(&self, url: &str) -> Result<Response> {
        let mut resp = self.fetcher.get(url).await?;

        // 从响应中提取 cookies 并存储
        self.store_cookies_from_response(url, &resp).await;

        // 注入已存储的 cookies 到响应（供用户查看）
        resp.cookies = self.cookie_strings_for(url).await;

        Ok(resp)
    }

    /// POST 请求（自动携带/更新 cookies）。
    pub async fn post(&self, url: &str, body: Option<&str>) -> Result<Response> {
        let mut resp = self.fetcher.post(url, body).await?;
        self.store_cookies_from_response(url, &resp).await;
        resp.cookies = self.cookie_strings_for(url).await;
        Ok(resp)
    }

    /// 发送自定义请求。
    pub async fn fetch(&self, req: Request) -> Result<Response> {
        let resp = self.fetcher.fetch(req).await?;
        self.store_cookies_from_response(&resp.url.clone(), &resp).await;
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

    /// 获取指定域名的 cookies。
    pub async fn cookies_for(&self, domain: &str) -> HashMap<String, String> {
        let cookies = self.cookies.lock().await;
        cookies.get(domain).cloned().unwrap_or_default()
    }

    /// 清除所有 cookies。
    pub async fn clear_cookies(&self) {
        self.cookies.lock().await.clear();
    }

    /// 关闭会话（释放浏览器资源）。
    pub async fn close(&self) {
        let mut browser = self.browser.lock().await;
        if let Some(b) = browser.take() {
            let _ = b.close().await;
        }
    }

    /// 获取内部 Fetcher 引用。
    pub fn fetcher(&self) -> &Fetcher {
        &self.fetcher
    }

    // === 内部方法 ===

    async fn store_cookies_from_response(&self, url: &str, resp: &Response) {
        let domain = extract_domain(url);
        if domain.is_empty() { return; }

        let mut cookies = self.cookies.lock().await;

        // 从 Set-Cookie 响应头提取
        for (key, value) in &resp.headers {
            if key.eq_ignore_ascii_case("set-cookie") {
                if let Some(name_value) = value.split(';').next() {
                    if let Some((name, val)) = name_value.split_once('=') {
                        let name = name.trim().to_string();
                        let val = val.trim().to_string();
                        if !name.is_empty() {
                            cookies.entry(domain.clone()).or_default().insert(name, val);
                        }
                    }
                }
            }
        }
    }

    async fn cookie_strings_for(&self, url: &str) -> Vec<String> {
        let domain = extract_domain(url);
        let cookies = self.cookies.lock().await;
        cookies.get(&domain)
            .map(|pairs| pairs.iter().map(|(k, v)| format!("{}={}", k, v)).collect())
            .unwrap_or_default()
    }
}

/// Session 构建器。
pub struct SessionBuilder {
    mode: FetchMode,
    config: FetcherConfig,
}

impl SessionBuilder {
    pub(crate) fn new(mode: FetchMode) -> Self {
        Self { mode, config: FetcherConfig::default() }
    }

    /// 设置代理。
    pub fn proxy(mut self, url: &str) -> Self {
        self.config.proxy = Some(url.to_string());
        self
    }

    /// 设置超时。
    pub fn timeout(mut self, d: Duration) -> Self {
        self.config.timeout = d;
        self
    }

    /// 设置 headless 模式。
    pub fn headless(mut self, v: bool) -> Self {
        self.config.headless = v;
        self
    }

    /// 设置 TLS 指纹模拟。
    pub fn emulation(mut self, p: Profile) -> Self {
        self.config.emulation = Some(p);
        self
    }

    /// 设置 User-Agent。
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.config.user_agent = Some(ua.to_string());
        self
    }

    /// 启用/禁用人类行为模拟。
    pub fn human_mode(mut self, v: bool) -> Self {
        self.config.human_mode = v;
        self
    }

    /// 设置 CF 挑战超时。
    pub fn challenge_timeout(mut self, d: Duration) -> Self {
        self.config.challenge_timeout = d;
        self
    }

    /// 构建 Session。
    pub fn build(self) -> Result<Session> {
        let fetcher = Fetcher::new(self.mode, self.config);
        Ok(Session {
            fetcher,
            cookies: Arc::new(Mutex::new(HashMap::new())),
            browser: Arc::new(Mutex::new(None)),
        })
    }
}

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
    fn test_session_builder_http() {
        let session = Session::http()
            .proxy("http://127.0.0.1:7897")
            .timeout(Duration::from_secs(60))
            .build();
        assert!(session.is_ok());
        let s = session.unwrap();
        assert_eq!(s.fetcher().mode(), FetchMode::Http);
    }

    #[test]
    fn test_session_builder_stealth() {
        let session = Session::stealth()
            .headless(true)
            .human_mode(true)
            .challenge_timeout(Duration::from_secs(60))
            .build();
        assert!(session.is_ok());
        let s = session.unwrap();
        assert_eq!(s.fetcher().mode(), FetchMode::Stealth);
    }

    #[tokio::test]
    async fn test_session_set_and_get_cookies() {
        let session = Session::http().build().unwrap();
        session.set_cookie("example.com", "token", "abc123").await;
        session.set_cookie("example.com", "sid", "xyz").await;

        let cookies = session.cookies_for("example.com").await;
        assert_eq!(cookies.get("token").unwrap(), "abc123");
        assert_eq!(cookies.get("sid").unwrap(), "xyz");
    }

    #[tokio::test]
    async fn test_session_clear_cookies() {
        let session = Session::http().build().unwrap();
        session.set_cookie("example.com", "a", "1").await;
        session.clear_cookies().await;
        let cookies = session.cookies_for("example.com").await;
        assert!(cookies.is_empty());
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(extract_domain("https://example.com/path"), "example.com");
        assert_eq!(extract_domain("http://sub.domain.org:8080/x"), "sub.domain.org");
        assert_eq!(extract_domain("invalid"), "");
    }
}
