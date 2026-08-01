//! Proxy URL parsing and configuration for the fetch client.

use std::fmt;

/// Parsed proxy configuration (HTTP client 内部使用)。
///
/// 区别于 `crate::config::ProxyConfig`（浏览器启动参数）。
#[derive(Clone)]
pub struct ParsedProxy {
    /// Full proxy URL (e.g., "http://user:pass@host:port")
    pub url: String,
    /// Proxy host
    pub host: String,
    /// Proxy port
    pub port: u16,
    /// Optional username
    pub username: Option<String>,
    /// Optional password
    pub password: Option<String>,
}

impl fmt::Debug for ParsedProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 脱敏：URL 和 password 可能包含凭据，不直接输出
        let masked_url = if self.username.is_some() {
            format!(
                "{}://***@{}:{}",
                self.url.split("://").next().unwrap_or("http"),
                self.host,
                self.port
            )
        } else {
            self.url.clone()
        };
        f.debug_struct("ParsedProxy")
            .field("url", &masked_url)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

impl ParsedProxy {
    /// Parse a proxy URL string into a ParsedProxy.
    ///
    /// Supported formats:
    /// - `http://host:port`
    /// - `http://user:pass@host:port`
    /// - `socks5://host:port`
    pub fn parse(url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_string();
        let port = parsed.port().unwrap_or(1080);
        let username = if parsed.username().is_empty() {
            None
        } else {
            Some(parsed.username().to_string())
        };
        let password = parsed.password().map(|p| p.to_string());

        Some(Self {
            url: url.to_string(),
            host,
            port,
            username,
            password,
        })
    }

    /// Format as a wreq-compatible proxy URL.
    pub fn to_proxy_url(&self) -> String {
        self.url.clone()
    }
}

/// Convert a list of proxy strings to ParsedProxy list.
pub fn parse_proxies(proxies: &[String]) -> Vec<ParsedProxy> {
    proxies
        .iter()
        .filter_map(|p| ParsedProxy::parse(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let cfg = ParsedProxy::parse("http://proxy.example.com:8080").unwrap();
        assert_eq!(cfg.host, "proxy.example.com");
        assert_eq!(cfg.port, 8080);
        assert!(cfg.username.is_none());
        assert!(cfg.password.is_none());
    }

    #[test]
    fn test_parse_with_auth() {
        let cfg = ParsedProxy::parse("http://user:pass@proxy.example.com:3128").unwrap();
        assert_eq!(cfg.host, "proxy.example.com");
        assert_eq!(cfg.port, 3128);
        assert_eq!(cfg.username, Some("user".to_string()));
        assert_eq!(cfg.password, Some("pass".to_string()));
    }

    #[test]
    fn test_parse_socks5() {
        let cfg = ParsedProxy::parse("socks5://127.0.0.1:1080").unwrap();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 1080);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(ParsedProxy::parse("not-a-url").is_none());
    }
}
