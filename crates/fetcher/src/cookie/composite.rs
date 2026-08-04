//! Composite cookie jar — shared persistent HTTP/CF state behind one seam.

use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use url::Url;

#[cfg(feature = "stealth")]
use super::CfCookieJar;
use super::{Cookie, CookieJar, HttpCookieJar};

/// 复合 cookie jar：HTTP 与 CF 会话共享同一个 trait seam。
///
/// Browser cookie 状态需要 CDP session，由抓取路径在 page 上下文中读写；
/// 持久化的 HTTP/CF 状态统一走这里。
pub struct CompositeCookieJar {
    http: Arc<HttpCookieJar>,
    #[cfg(feature = "stealth")]
    cf: Arc<CfCookieJar>,
}

#[cfg(feature = "stealth")]
impl CompositeCookieJar {
    /// 创建包含 HTTP 与 CF 后端的复合 jar。
    #[must_use]
    pub fn new(http: Arc<HttpCookieJar>, cf: Arc<CfCookieJar>) -> Self {
        Self { http, cf }
    }
}

#[cfg(not(feature = "stealth"))]
impl CompositeCookieJar {
    /// 创建仅包含 HTTP 后端的复合 jar。
    #[must_use]
    pub fn new(http: Arc<HttpCookieJar>) -> Self {
        Self { http }
    }
}

#[async_trait]
impl CookieJar for CompositeCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        let mut cookies = self.http.get(url).await;
        #[cfg(feature = "stealth")]
        cookies.extend(self.cf.get(url).await);
        let mut seen = HashSet::new();
        cookies.retain(|c| seen.insert((c.name.clone(), c.domain.clone(), c.path.clone())));
        cookies
    }

    async fn set(&self, cookie: Cookie) {
        self.http.set(cookie.clone()).await;
        #[cfg(feature = "stealth")]
        self.cf.set(cookie).await;
    }

    async fn set_batch(&self, cookies: Vec<Cookie>) {
        self.http.set_batch(cookies.clone()).await;
        #[cfg(feature = "stealth")]
        self.cf.set_batch(cookies).await;
    }

    async fn clear(&self, url: &Url) {
        self.http.clear(url).await;
        #[cfg(feature = "stealth")]
        self.cf.clear(url).await;
    }

    async fn ua(&self, url: &Url) -> Option<String> {
        #[cfg(feature = "stealth")]
        {
            return self.cf.ua(url).await;
        }
        #[cfg(not(feature = "stealth"))]
        {
            let _ = url;
            None
        }
    }

    async fn set_session_ua(&self, domain: &str, ua: Option<&str>) {
        #[cfg(feature = "stealth")]
        self.cf.set_session_ua(domain, ua).await;
        #[cfg(not(feature = "stealth"))]
        let _ = (domain, ua);
    }
}
