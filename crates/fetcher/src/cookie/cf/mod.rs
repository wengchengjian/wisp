//! CF 会话 cookie jar — moka::Cache + 文件持久化。
//!
//! ARCH: 从 FetchClient 迁出，由 StealthStrategy 持有。
//! 保留两级缓存（moka 内存 + JSON 文件），TTL 默认 30 分钟。

mod jar;
mod session;

pub use jar::CfCookieJar;
pub use session::CfSession;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cookie::{Cookie, CookieJar};
    use std::time::Duration;
    use url::Url;

    fn make_url(s: &str) -> Url {
        Url::parse(s).expect("合法 URL")
    }

    /// 创建临时目录 + CfCookieJar。
    fn make_jar() -> (CfCookieJar, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let jar = CfCookieJar::new(dir.path(), Duration::from_mins(1));
        (jar, dir)
    }

    #[tokio::test]
    async fn cf_set_and_get_cookie() {
        let (jar, _dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "abc123".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            same_site: Some("Lax".into()),
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/path");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "cf_clearance");
        assert_eq!(cookies[0].value, "abc123");
    }

    #[tokio::test]
    async fn cf_clear_removes_session() {
        let (jar, _dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/");
        assert!(!jar.get(&url).await.is_empty());

        jar.clear(&url).await;
        assert!(jar.get(&url).await.is_empty());
    }

    #[tokio::test]
    async fn cf_header_returns_string() {
        let (jar, _dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "token123".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/");
        let header = jar.header(&url).await;
        assert_eq!(header.as_deref(), Some("cf_clearance=token123"));
    }

    #[tokio::test]
    async fn cf_persist_and_reload() {
        let (jar, dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "persisted".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        // 文件应存在
        let file_path = dir.path().join("cf_sessions.json");
        assert!(file_path.exists(), "持久化文件应存在");

        // 重新加载，cookie 应恢复（TTL 内）
        let jar2 = CfCookieJar::new(dir.path(), Duration::from_mins(1));
        let url = make_url("https://example.com/");
        let cookies = jar2.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].value, "persisted");
    }

    #[tokio::test]
    async fn cf_expired_session_not_reloaded() {
        let (jar, dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "expired".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        // 用极短 TTL 重新加载，文件中的 saved_at 已过期
        let jar2 = CfCookieJar::new(dir.path(), Duration::from_millis(1));
        tokio::time::sleep(Duration::from_millis(10)).await;
        let url = make_url("https://example.com/");
        // 注意：moka 自身的 TTL 在加载时已生效，过期条目不会进入 moka
        let cookies = jar2.get(&url).await;
        // 加载时 saved_at 与当前时间差 > 1ms，应被跳过
        assert!(cookies.is_empty(), "过期会话不应被加载");
    }

    #[tokio::test]
    async fn cf_set_replaces_same_name() {
        let (jar, _dir) = make_jar();
        let c1 = Cookie {
            name: "x".into(),
            value: "v1".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        let c2 = Cookie {
            name: "x".into(),
            value: "v2".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(c1).await;
        jar.set(c2).await;

        let url = make_url("https://example.com/");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1, "同名 cookie 应被替换");
        assert_eq!(cookies[0].value, "v2");
    }

    #[tokio::test]
    async fn cf_ua_returns_session_ua() {
        let (jar, _dir) = make_jar();
        jar.insert_session(
            "example.com".into(),
            CfSession {
                cookies: Vec::new(),
                ua: "Mozilla/5.0 Chrome/136".into(),
                saved_at: chrono::Utc::now().timestamp(),
            },
        );

        let url = make_url("https://example.com/");
        assert_eq!(
            jar.ua(&url).await.as_deref(),
            Some("Mozilla/5.0 Chrome/136")
        );
    }

    #[tokio::test]
    async fn cf_ua_none_when_session_has_no_ua() {
        let (jar, _dir) = make_jar();
        let cookie = Cookie {
            name: "cf_clearance".into(),
            value: "token".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;

        let url = make_url("https://example.com/");
        assert!(jar.ua(&url).await.is_none(), "空 UA 不应返回");
    }
}
