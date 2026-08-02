//! SSRF 防护：URL 校验工具。
//!
//! 提供 [`validate_url`] 校验 URL scheme + host，并拒绝内网/环回/链路本地地址。
//! 适用于 MCP 等接受外部 URL 输入的场景（ND-003-SEC）。

use std::net::IpAddr;

use crate::error::{McpError, Result, WispError};

/// 校验 URL 是否安全可访问。
///
/// 校验规则：
/// 1. 能被 `url::Url::parse` 解析（处理大小写、前导空格等）
/// 2. scheme 仅允许 `http` / `https`
/// 3. host 非空
/// 4. 若 host 是 IP，拒绝内网/环回/链路本地/多播地址
///
/// # 示例
///
/// ```
/// use wisp_core::utils::ssrf::validate_url;
///
/// assert!(validate_url("https://example.com").is_ok());
/// assert!(validate_url("http://127.0.0.1/").is_err()); // Err（环回）
/// assert!(validate_url("ftp://example.com").is_err()); // Err（scheme 非法）
/// ```
fn parse_safe_url(url: &str) -> Result<url::Url> {
    url::Url::parse(url.trim())
        .map_err(|e| WispError::Mcp(McpError::General(format!("URL 解析失败 '{url}': {e}"))))
}

fn check_scheme(parsed: &url::Url, url: &str) -> Result<()> {
    if parsed.scheme() == "http" || parsed.scheme() == "https" {
        return Ok(());
    }
    Err(WispError::Mcp(McpError::General(format!(
        "URL scheme 非法，仅支持 http/https，拒绝: {url}"
    ))))
}

fn check_host(parsed: &url::Url, url: &str) -> Result<()> {
    let host_str = parsed.host_str();
    if host_str.map(|s| s.is_empty()).unwrap_or(true) {
        return Err(WispError::Mcp(McpError::General(format!(
            "URL 缺少 host: {url}"
        ))));
    }
    parsed
        .host()
        .ok_or_else(|| WispError::Mcp(McpError::General(format!("URL 缺少 host: {url}"))))?;
    Ok(())
}

fn reject_private_host(parsed: &url::Url, url: &str) -> Result<()> {
    let host = parsed
        .host()
        .ok_or_else(|| WispError::Mcp(McpError::General(format!("URL 缺少 host: {url}"))))?;
    match host {
        url::Host::Ipv4(ip) if is_private_ip(&IpAddr::V4(ip)) => {
            Err(WispError::Mcp(McpError::General(format!(
                "URL host 指向内网/保留 IP 地址，拒绝: {url} ({ip})"
            ))))
        }
        url::Host::Ipv6(ip) if is_private_ip(&IpAddr::V6(ip)) => {
            Err(WispError::Mcp(McpError::General(format!(
                "URL host 指向内网/保留 IPv6 地址，拒绝: {url} ({ip})"
            ))))
        }
        _ => Ok(()),
    }
}

pub fn validate_url(url: &str) -> Result<()> {
    let parsed = parse_safe_url(url)?;
    check_scheme(&parsed, url)?;
    check_host(&parsed, url)?;
    reject_private_host(&parsed, url)
}

fn is_private_ipv4(v4: std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_multicast()
        || v4.is_broadcast()
}

fn is_private_ipv6(v6: std::net::Ipv6Addr) -> bool {
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        || (v6.segments()[0] & 0xfe00) == 0xfc00
        || (v6.segments()[0] & 0xffc0) == 0xfe80
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(*v4),
        IpAddr::V6(v6) => is_private_ipv6(*v6),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_public_urls() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/path").is_ok());
        assert!(validate_url("  https://example.com  ").is_ok()); // 前导空格
        assert!(validate_url("HTTPS://example.com").is_ok()); // 大小写
    }

    #[test]
    fn rejects_invalid_scheme() {
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:void(0)").is_err());
    }

    #[test]
    fn rejects_loopback() {
        assert!(validate_url("http://127.0.0.1/").is_err());
        assert!(validate_url("http://localhost/").is_ok()); // localhost 非 IP，放行（DNS 解析时再判断）
        assert!(validate_url("http://[::1]/").is_err());
    }

    #[test]
    fn rejects_private_ip() {
        assert!(validate_url("http://10.0.0.1/").is_err());
        assert!(validate_url("http://192.168.1.1/").is_err());
        assert!(validate_url("http://172.16.0.1/").is_err());
    }

    #[test]
    fn rejects_link_local() {
        assert!(validate_url("http://169.254.169.254/").is_err()); // AWS metadata
        assert!(validate_url("http://[fe80::1]/").is_err());
    }

    #[test]
    fn rejects_missing_host() {
        assert!(validate_url("http://").is_err());
        // 注：`https:///path` 被 url crate 解析为 host=Domain("path")，
        // 不会失败。这不是 missing host 案例，故不在此测试。
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(128))]
        #[test]
        fn public_hostnames_are_accepted(host in "[a-z0-9]{1,20}\\.example\\.com", path in "[a-zA-Z0-9/._-]{0,60}") {
            let url = format!("https://{host}/{path}");
            assert!(validate_url(&url).is_ok(), "public URL should pass: {url}");
        }

        #[test]
        fn private_ipv4_is_rejected(a in 0u8..=255, b in 0u8..=255, c in 0u8..=255, d in 0u8..=255) {
            let ip = std::net::Ipv4Addr::new(a, b, c, d);
            if ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
                let url = format!("http://{ip}/");
                assert!(validate_url(&url).is_err(), "private IP should fail: {url}");
            }
        }
    }
}
