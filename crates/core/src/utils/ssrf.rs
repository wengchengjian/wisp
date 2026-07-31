//! SSRF 防护：URL 校验工具。
//!
//! 提供 [`validate_url`] 校验 URL scheme + host，并拒绝内网/环回/链路本地地址。
//! 适用于 MCP 等接受外部 URL 输入的场景（ND-003-SEC）。

use std::net::IpAddr;

use crate::error::{Result, WispError, McpError};

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
/// ```ignore
/// use crate::utils::ssrf::validate_url;
///
/// validate_url("https://example.com")?;       // OK
/// validate_url("http://127.0.0.1/")?;          // Err（环回）
/// validate_url("ftp://example.com")?;          // Err（scheme 非法）
/// ```
pub fn validate_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    let parsed = url::Url::parse(trimmed).map_err(|e| {
        WispError::Mcp(McpError::General(format!("URL 解析失败 '{url}': {e}")))
    })?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(WispError::Mcp(McpError::General(
            format!("URL scheme 非法，仅支持 http/https，拒绝: {url}")
        )));
    }

    // 使用 host() 而非 host_str()，正确处理 IPv6 字面量（host_str 返回 "[::1]" 带括号）
    // 注意：url crate 会将 "https:///path" 解析为 host=Some("")（空 domain），
    // 因此需要额外检查 host_str 是否为空字符串。
    let host_str = parsed.host_str();
    if host_str.map(|s| s.is_empty()).unwrap_or(true) {
        return Err(WispError::Mcp(McpError::General(format!(
            "URL 缺少 host: {url}"
        ))));
    }
    let host = parsed.host().ok_or_else(|| {
        WispError::Mcp(McpError::General(format!("URL 缺少 host: {url}")))
    })?;

    // 若 host 是 IP，检查是否为内网/保留地址
    if let url::Host::Ipv4(ip) = host {
        if is_private_ip(&IpAddr::V4(ip)) {
            return Err(WispError::Mcp(McpError::General(
                format!("URL host 指向内网/保留 IP 地址，拒绝: {url} ({ip})")
            )));
        }
    } else if let url::Host::Ipv6(ip) = host {
        if is_private_ip(&IpAddr::V6(ip)) {
            return Err(WispError::Mcp(McpError::General(
                format!("URL host 指向内网/保留 IPv6 地址，拒绝: {url} ({ip})")
            )));
        }
    }

    Ok(())
}

/// 判断 IP 是否为内网/环回/链路本地/多播/保留地址。
///
/// - `127.0.0.0/8`、`::1` — 环回
/// - `10.0.0.0/8`、`172.16.0.0/12`、`192.168.0.0/16`、`fc00::/7` — 私有
/// - `169.254.0.0/16`、`fe80::/10` — 链路本地
/// - `0.0.0.0/8` — 未指定/本机
/// - `224.0.0.0/4`、`ff00::/8` — 多播
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // IPv6 私有地址 fc00::/7（unique local）
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // IPv6 链路本地 fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
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
}
