//! 浏览器 cookie jar — 通过 CDP Network.getCookies/setCookie/clearBrowserCookies。
//!
//! ARCH: 每个 Page 持有一个 BrowserCookieJar，导航后可读取 cookie，
//! ChallengeSolver 解决 CF 后将 cookie 写入此 jar。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use url::Url;

use crate::browser::cdp::CdpSession;
use crate::cookie::{Cookie, CookieJar};
use crate::error::Result;

/// 浏览器 cookie jar（通过 CDP）。
///
/// `session_id = None` 时作用于 browser level（所有 target 共享）；
/// `Some(id)` 时仅作用于该 target（页面）。
pub struct BrowserCookieJar {
    session: Arc<CdpSession>,
    session_id: Option<String>,
}

impl BrowserCookieJar {
    /// 创建 browser-level jar（无 session_id 隔离）。
    #[must_use]
    pub fn new_browser_level(session: Arc<CdpSession>) -> Self {
        Self {
            session,
            session_id: None,
        }
    }

    /// 创建 target-level jar（绑定特定 page session）。
    #[must_use]
    pub fn new_for_target(session: Arc<CdpSession>, session_id: String) -> Self {
        Self {
            session,
            session_id: Some(session_id),
        }
    }

    /// 执行 CDP 命令（带 session_id 如果有）。
    async fn cmd(&self, method: &str, params: Value) -> Result<Value> {
        self.session
            .execute_with_session(method, params, self.session_id.as_deref())
            .await
    }

    /// 从 CDP Network.getCookies 返回的 JSON 转 Cookie。
    fn value_to_cookie(v: &Value, default_domain: &str) -> Option<Cookie> {
        Some(Cookie {
            name: v.get("name")?.as_str()?.to_string(),
            value: v.get("value")?.as_str()?.to_string(),
            domain: v
                .get("domain")
                .and_then(|d| d.as_str())
                .unwrap_or(default_domain)
                .to_string(),
            path: v
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("/")
                .to_string(),
            secure: v
                .get("secure")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            http_only: v
                .get("httpOnly")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            same_site: v
                .get("sameSite")
                .and_then(|s| s.as_str())
                .map(std::string::ToString::to_string),
            expires: v.get("expires").and_then(Value::as_f64),
        })
    }
}

#[async_trait]
impl CookieJar for BrowserCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let urls = vec![url.as_str()];
        let result = match self
            .cmd(
                "Network.getCookies",
                json!({ "urls": urls }),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("BrowserCookieJar::get Network.getCookies failed: {e}");
                return Vec::new();
            }
        };

        let host = url.host_str().unwrap_or("");
        result
            .get("cookies")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| Self::value_to_cookie(v, host))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn set(&self, cookie: Cookie) {
        let params = json!({
            "name": cookie.name,
            "value": cookie.value,
            "domain": cookie.domain,
            "path": cookie.path,
            "secure": cookie.secure,
            "httpOnly": cookie.http_only,
            "sameSite": cookie.same_site.clone().unwrap_or_else(|| "Lax".into()),
        });
        let params = if let Some(expires) = cookie.expires {
            let mut p = params;
            p["expires"] = json!(expires);
            p
        } else {
            params
        };

        if let Err(e) = self.cmd("Network.setCookie", params).await {
            tracing::warn!("BrowserCookieJar::set Network.setCookie failed: {e}");
        }
    }

    async fn clear(&self, url: &Url) {
        // Network.clearBrowserCookies 清除所有 cookie（无 url 过滤），
        // 注意：这会清除所有域名的 cookie，仅用于失效会话场景。
        let _ = self.cmd("Network.clearBrowserCookies", json!({})).await;
        tracing::debug!("BrowserCookieJar::clear cleared all browser cookies (url={url})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_to_cookie_extracts_fields() {
        let v = json!({
            "name": "session",
            "value": "abc",
            "domain": "example.com",
            "path": "/",
            "secure": true,
            "httpOnly": true,
            "sameSite": "Lax",
            "expires": 1234567890.0,
        });
        let cookie = BrowserCookieJar::value_to_cookie(&v, "fallback").expect("完整字段应解析成功");
        assert_eq!(cookie.name, "session");
        assert_eq!(cookie.value, "abc");
        assert_eq!(cookie.domain, "example.com");
        assert_eq!(cookie.path, "/");
        assert!(cookie.secure);
        assert!(cookie.http_only);
        assert_eq!(cookie.same_site.as_deref(), Some("Lax"));
        assert_eq!(cookie.expires, Some(1234567890.0));
    }

    #[test]
    fn value_to_cookie_uses_default_domain_when_missing() {
        let v = json!({
            "name": "x",
            "value": "y",
        });
        let cookie = BrowserCookieJar::value_to_cookie(&v, "default.com").expect("仅 name/value 也应解析");
        assert_eq!(cookie.domain, "default.com");
        assert_eq!(cookie.path, "/");
        assert!(!cookie.secure);
        assert!(!cookie.http_only);
        assert!(cookie.same_site.is_none());
        assert!(cookie.expires.is_none());
    }

    #[test]
    fn value_to_cookie_returns_none_for_missing_name() {
        let v = json!({ "value": "y" });
        assert!(BrowserCookieJar::value_to_cookie(&v, "x").is_none());
    }

    // === 集成测试（需要 Chrome 环境） ===

    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn browser_set_and_get_cookie_roundtrip() {
        use crate::browser::Browser;
        use crate::config::LaunchOptions;

        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");
        // 先导航到一个真实 URL 让 cookie 有 domain 上下文
        page.goto("data:text/html,<html></html>")
            .await
            .expect("导航");

        let jar = BrowserCookieJar::new_for_target(
            Arc::clone(&page.session),
            page.session_id.clone(),
        );
        jar.set(Cookie {
            name: "test_cookie".into(),
            value: "value123".into(),
            domain: "localhost".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: Some("Lax".into()),
            expires: None,
        })
        .await;

        let url = Url::parse("http://localhost/").expect("合法 URL");
        let cookies = jar.get(&url).await;
        assert!(cookies.iter().any(|c| c.name == "test_cookie" && c.value == "value123"));

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }

    #[tokio::test]
    #[ignore = "需要 Chrome 浏览器环境"]
    async fn browser_clear_removes_cookies() {
        use crate::browser::Browser;
        use crate::config::LaunchOptions;

        let browser = Browser::launch(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .await
        .expect("启动浏览器");

        let mut page = browser.new_page().await.expect("创建页面");
        page.goto("data:text/html,<html></html>")
            .await
            .expect("导航");

        let jar = BrowserCookieJar::new_for_target(
            Arc::clone(&page.session),
            page.session_id.clone(),
        );
        jar.set(Cookie {
            name: "to_clear".into(),
            value: "v".into(),
            domain: "localhost".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = Url::parse("http://localhost/").expect("合法 URL");
        jar.clear(&url).await;
        // clearBrowserCookies 清除所有 cookie
        let cookies = jar.get(&url).await;
        assert!(cookies.iter().all(|c| c.name != "to_clear"));

        let _ = page.close().await;
        browser.close().await.expect("关闭浏览器");
    }
}
