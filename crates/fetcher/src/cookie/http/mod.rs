//! HTTP cookie jar 模块。

mod jar;

pub use jar::HttpCookieJar;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cookie::{Cookie, CookieJar};
    use url::Url;

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

    #[tokio::test]
    async fn http_ua_returns_none() {
        let jar = HttpCookieJar::new();
        let url = make_url("https://example.com/");
        assert!(jar.ua(&url).await.is_none(), "HTTP jar 不绑定 UA");
    }
}
