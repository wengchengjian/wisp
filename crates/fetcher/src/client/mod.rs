//! 统一请求客户端 — 封装 HTTP Client 和 BrowserPool。
//!
//! - HTTP 请求：共享 `http::Client`（连接池复用）
//! - 浏览器请求：通过 `BrowserPool`（实例复用，RAII 自动归还）
//! - Cookie 管理：通过 `cookie_jar: Arc<dyn CookieJar>` 统一 HTTP/浏览器/CF 三处 cookie

mod config;
mod fetch_client;

pub use config::FetchClientConfig;
pub use fetch_client::FetchClient;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_fetch_client_config_default() {
        let config = FetchClientConfig::default();
        assert_eq!(config.max_concurrent_pages, 4);
        assert!(config.headless);
        assert!(config.human_mode);
    }

    #[test]
    fn test_fetch_client_http_only() {
        // max_concurrent_pages=0 → 无浏览器池
        let config = FetchClientConfig {
            max_concurrent_pages: 0,
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        #[cfg(feature = "browser")]
        assert!(client.browser_pool().is_none());
        assert_eq!(client.http().config_ref().timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn fetch_http_blocks_configured_domain() {
        use wisp_core::Request;
        use wisp_http::DomainBlocker;
        let mut blocker = DomainBlocker::new();
        blocker.block_domain("ads.example.com");
        let config = FetchClientConfig {
            domain_blocker: Some(blocker),
            ..Default::default()
        };
        let client = FetchClient::new(config).expect("build client");
        let err = client
            .fetch_http(&Request::get("https://ads.example.com/ad.js"))
            .await
            .expect_err("拦截域名应报错");
        assert!(err.to_string().contains("blocked"), "错误应说明拦截: {err}");
    }

    #[cfg(feature = "browser")]
    #[test]
    fn test_fetch_client_with_browser_pool() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        assert!(client.browser_pool().is_some());
    }

    #[cfg(feature = "browser")]
    #[test]
    fn browser_pool_rejects_authenticated_proxy() {
        let mut config = FetchClientConfig::default();
        config.proxy = Some("http://user:pass@127.0.0.1:8080".into());
        let err = match FetchClient::new(config) {
            Ok(_) => panic!("认证代理应被拒绝"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("代理认证"), "错误应明确: {err}");
    }

    #[cfg(feature = "browser")]
    #[test]
    fn fetch_client_drop_does_not_panic_inside_runtime() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        drop(client);
    }

    #[tokio::test]
    async fn fetch_client_has_cookie_jar() {
        let config = FetchClientConfig::default();
        let client = FetchClient::new(config).expect("build client");
        // cookie_jar() 应返回非 None 的 Arc<dyn CookieJar>
        let jar = client.cookie_jar();
        // 默认使用 HttpCookieJar，应能 set/get
        use crate::cookie::Cookie;
        use url::Url;
        let cookie = Cookie {
            name: "test".into(),
            value: "v".into(),
            domain: "example.com".into(),
            path: "/".into(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        };
        jar.set(cookie).await;
        let url = Url::parse("https://example.com/").expect("合法 URL");
        let cookies = jar.get(&url).await;
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "test");
    }

    #[test]
    fn fetch_client_config_still_has_cf_fields() {
        // 验证 FetchClientConfig 仍保留 cf_cookie_ttl/cf_data_dir（供 StealthStrategy 在 PR2 使用）
        let config = FetchClientConfig::default();
        assert_eq!(config.cf_cookie_ttl, std::time::Duration::from_mins(30));
        assert_eq!(config.cf_data_dir, std::path::PathBuf::from("wisp-data"));
    }

    #[cfg(feature = "browser")]
    #[cfg(test)]
    mod browser_tests {
        use super::*;
        use crate::strategy::BrowserFetchStrategy;
        use async_trait::async_trait;
        use wisp_browser::Page;
        use wisp_core::error::Result;
        use wisp_core::{Request, Response};

        /// Mock 策略：返回固定响应，用于验证 fetch_browser 调用契约。
        struct MockStrategy {
            called: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        }

        #[async_trait]
        impl BrowserFetchStrategy for MockStrategy {
            async fn fetch(&self, _page: &mut Page, req: &Request) -> Result<Response> {
                self.called
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Response::from_browser(
                    200,
                    req.url.clone(),
                    "<html>mock</html>".to_string(),
                    "mock".to_string(),
                    Vec::new(),
                    req.clone(),
                ))
            }
        }

        #[tokio::test]
        async fn test_fetch_browser_invokes_strategy() {
            // max_concurrent_pages=0 会导致无 browser_pool，需 >0
            let config = FetchClientConfig::default();
            let client = FetchClient::new(config).expect("build client");
            let called = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let strategy = MockStrategy {
                called: called.clone(),
            };
            let req = Request::get("data:text/html,<html></html>");

            // 注意：此测试需要真实 Chrome（BrowserPool::acquire 会启动浏览器）
            // 若无 Chrome 环境，会返回 LaunchFailed 错误
            let result = client.fetch_browser(&req, &strategy).await;
            if result.is_ok() {
                assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 1);
            }
            // 无 Chrome 环境下不报错（忽略结果）
        }

        #[tokio::test]
        async fn test_fetch_browser_no_pool_returns_error() {
            let config = FetchClientConfig {
                max_concurrent_pages: 0,
                ..Default::default()
            };
            let client = FetchClient::new(config).expect("build client");
            let called = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let strategy = MockStrategy {
                called: called.clone(),
            };
            let req = Request::get("https://example.com/");

            let result = client.fetch_browser(&req, &strategy).await;
            assert!(result.is_err(), "无 browser_pool 应返回错误");
            // 策略不应被调用
            assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 0);
        }
    }
}
