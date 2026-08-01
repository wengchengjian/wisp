//! HTTP cookie jar — 包装 wreq::cookie::Jar。

use std::sync::Arc;

use async_trait::async_trait;
use url::Url;

use crate::cookie::{Cookie, CookieJar};

pub struct HttpCookieJar {
    jar: Arc<wreq::cookie::Jar>,
}

impl HttpCookieJar {
    /// 创建空 jar。
    #[must_use]
    pub fn new() -> Self {
        Self {
            jar: Arc::new(wreq::cookie::Jar::default()),
        }
    }

    /// 暴露内部 jar 供 wreq::Client::builder().cookie_provider() 使用。
    ///
    /// 用法：
    /// ```ignore
    /// let http_jar = Arc::new(HttpCookieJar::new());
    /// let client = wreq::Client::builder()
    ///     .cookie_provider(http_jar.jar())
    ///     .build()?;
    /// ```
    #[must_use]
    pub fn jar(&self) -> Arc<wreq::cookie::Jar> {
        Arc::clone(&self.jar)
    }
}

impl Default for HttpCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CookieJar for HttpCookieJar {
    async fn get(&self, url: &Url) -> Vec<Cookie> {
        // wreq::cookie::Jar 不暴露按 uri 返回 Vec<Cookie> 的 API，
        // 使用 get_all + 手动按 domain/path 过滤，保持 trait 语义清晰。
        let host = url.host_str().unwrap_or("");
        let path = url.path();
        self.jar
            .get_all()
            .filter(|c| {
                let domain_match = c.domain().is_some_and(|d| host.ends_with(d));
                let path_match = c.path().is_none_or(|p| path.starts_with(p));
                domain_match && path_match
            })
            .map(|c| Cookie {
                name: c.name().to_string(),
                value: c.value().to_string(),
                domain: c.domain().unwrap_or(host).to_string(),
                path: c.path().unwrap_or("/").to_string(),
                secure: c.secure(),
                http_only: c.http_only(),
                same_site: if c.same_site_lax() {
                    Some("Lax".into())
                } else if c.same_site_strict() {
                    Some("Strict".into())
                } else {
                    None
                },
                expires: c.expires().map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map_or(0.0, |d| d.as_secs_f64())
                }),
            })
            .collect()
    }

    async fn set(&self, cookie: Cookie) {
        // expires 是 Unix 时间戳（秒）。wreq::cookie::Jar 的 Cookie::expires() 仅读
        // cookie::Cookie 的 expires 字段（Expiration 类型），不读 max_age 字段，
        // 因此用 `Expires=<HTTP-date>` 而非 `Max-Age=<secs>`，否则 expires 信息丢失。
        // 过期 cookie（expires <= now）直接跳过。
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let expires_str = match cookie.expires {
            Some(expires) if expires > now => {
                // RFC 7231 IMF-fixdate: "Wed, 21 Oct 2015 07:28:00 GMT"
                chrono::DateTime::<chrono::Utc>::from_timestamp(expires as i64, 0)
                    .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
            }
            Some(_) => return, // 已过期
            None => None,      // session cookie
        };

        // 构造 Set-Cookie 字符串注入到 wreq::cookie::Jar
        let mut cookie_str = format!("{}={}", cookie.name, cookie.value);
        cookie_str.push_str(&format!("; Domain={}", cookie.domain));
        cookie_str.push_str(&format!("; Path={}", cookie.path));
        if cookie.secure {
            cookie_str.push_str("; Secure");
        }
        if cookie.http_only {
            cookie_str.push_str("; HttpOnly");
        }
        if let Some(ref ss) = cookie.same_site {
            cookie_str.push_str(&format!("; SameSite={ss}"));
        }
        if let Some(ref exp) = expires_str {
            cookie_str.push_str(&format!("; Expires={exp}"));
        }
        // 使用 domain 构造关联 uri（Jar 会从中提取 host 并校验 domain-match）
        let uri = format!("https://{}/", cookie.domain);
        self.jar.add(cookie_str.as_str(), &uri);
    }

    async fn clear(&self, url: &Url) {
        // wreq::cookie::Jar 没有 clear-by-url，只能全清或按 name+path 删除。
        // 实现：收集与 url host 匹配的 cookie，用其原始 domain/path 构造精确 URI 删除。
        let host = url.host_str().unwrap_or("");
        let to_remove: Vec<_> = self
            .jar
            .get_all()
            .filter(|c| c.domain().is_some_and(|d| host.ends_with(d)))
            .map(|c| {
                let domain = c.domain().unwrap_or(host).to_string();
                let path = c.path().unwrap_or("/").to_string();
                (c, domain, path)
            })
            .collect();
        for (cookie, domain, path) in to_remove {
            let uri = format!("https://{domain}{path}");
            // Jar::remove 接受 Into<RawCookie<'static>>，wreq::cookie::Cookie 满足此约束
            self.jar.remove(cookie, &uri);
        }
    }
}
