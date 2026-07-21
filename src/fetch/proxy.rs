//! Proxy URL parsing and configuration for the fetch client.

/// Parsed proxy configuration.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
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

impl ProxyConfig {
    /// Parse a proxy URL string into a ProxyConfig.
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

/// Convert a list of proxy strings to ProxyConfig list.
pub fn parse_proxies(proxies: &[String]) -> Vec<ProxyConfig> {
    proxies.iter().filter_map(|p| ProxyConfig::parse(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let cfg = ProxyConfig::parse("http://proxy.example.com:8080").unwrap();
        assert_eq!(cfg.host, "proxy.example.com");
        assert_eq!(cfg.port, 8080);
        assert!(cfg.username.is_none());
        assert!(cfg.password.is_none());
    }

    #[test]
    fn test_parse_with_auth() {
        let cfg = ProxyConfig::parse("http://user:pass@proxy.example.com:3128").unwrap();
        assert_eq!(cfg.host, "proxy.example.com");
        assert_eq!(cfg.port, 3128);
        assert_eq!(cfg.username, Some("user".to_string()));
        assert_eq!(cfg.password, Some("pass".to_string()));
    }

    #[test]
    fn test_parse_socks5() {
        let cfg = ProxyConfig::parse("socks5://127.0.0.1:1080").unwrap();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 1080);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(ProxyConfig::parse("not-a-url").is_none());
    }
}
