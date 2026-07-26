//! HTTP cookie jar — 包装 wreq::cookie::Jar。
//!
//! ARCH: HttpCookieJar 自创建 `wreq::cookie::Jar`，通过 `ClientBuilder::cookie_provider`
//! 注入到 wreq::Client，实现读写共享（HttpCookieJar 写入 → wreq::Client 自动携带）。
//! wreq::Client 6.0.0-rc.29 不暴露 cookie_store getter，因此采用注入式共享。

use std::sync::Arc;

use async_trait::async_trait;
use url::Url;

use crate::cookie::{Cookie, CookieJar};

/// HTTP cookie jar（包装 wreq::cookie::Jar）。
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_url(s: &str) -> Url {
        Url::parse(s).expect("合法 URL")
    }

    #[tokio::test]
    async fn http_set_and_get_cookie() {
        let jar = HttpCookieJar::new();
        let cookie = Cookie {
            name: "session".into(),
            value: "abc".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: Some("Lax".into()),
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/path");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "session");
        assert_eq!(cookies[0].value, "abc");
    }

    #[tokio::test]
    async fn http_header_returns_string() {
        let jar = HttpCookieJar::new();
        jar.set(Cookie {
            name: "a".into(),
            value: "1".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;
        jar.set(Cookie {
            name: "b".into(),
            value: "2".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert!(header.is_some());
        let header = header.expect("非空 header");
        assert!(header.contains("a=1"));
        assert!(header.contains("b=2"));
    }

    #[tokio::test]
    async fn http_clear_removes_matching() {
        let jar = HttpCookieJar::new();
        jar.set(Cookie {
            name: "x".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = make_url("https://example.com/");
        assert!(!jar.get(&url).await.is_empty());

        jar.clear(&url).await;
        assert!(jar.get(&url).await.is_empty(), "clear 后应无 cookie");
    }

    #[tokio::test]
    async fn http_jar_injectable_into_wreq_client() {
        // 验证 jar() 返回的 Arc<wreq::cookie::Jar> 可注入到 wreq::Client::builder()
        let http_jar = HttpCookieJar::new();
        let jar = http_jar.jar();
        let client = wreq::Client::builder().cookie_provider(jar).build();
        assert!(client.is_ok(), "应能注入到 wreq::Client");
    }

    #[tokio::test]
    async fn http_domain_filter() {
        let jar = HttpCookieJar::new();
        jar.set(Cookie {
            name: "a".into(),
            value: "1".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;
        jar.set(Cookie {
            name: "b".into(),
            value: "2".into(),
            domain: "other.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        })
        .await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "a");
    }

    #[tokio::test]
    async fn http_set_with_expires() {
        // 验证带 expires 的 cookie 能被正确存储并读回 expires 字段
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间在 1970 之后")
            .as_secs_f64();
        let jar = HttpCookieJar::new();
        // expires 设为 now + 3600s（1 小时后）
        jar.set(Cookie {
            name: "token".into(),
            value: "xyz".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: true,
            same_site: Some("Lax".into()),
            expires: Some(now + 3600.0),
        })
        .await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1, "应读回 1 个 cookie");
        assert_eq!(cookies[0].name, "token");
        assert_eq!(cookies[0].value, "xyz");
        // 读回的 expires 应为 Some 且接近入参（差异在 5 秒内视为精度可接受）
        let read_expires = cookies[0]
            .expires
            .expect("expires 应为 Some，非 session cookie");
        let delta = (read_expires - (now + 3600.0)).abs();
        assert!(
            delta < 5.0,
            "expires 偏差过大: {delta}s（读回 {read_expires}, 期望 {}）",
            now + 3600.0
        );
    }

    #[tokio::test]
    async fn http_set_with_expired_expires_skipped() {
        // 过期 cookie（expires <= now）应被跳过：jar 不存储，读回为空
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间在 1970 之后")
            .as_secs_f64();
        let jar = HttpCookieJar::new();
        jar.set(Cookie {
            name: "dead".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: Some(now - 10.0), // 已过期 10 秒
        })
        .await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert!(
            cookies.is_empty(),
            "过期 cookie 不应被存储，但读回 {len} 条",
            len = cookies.len()
        );
    }
}
