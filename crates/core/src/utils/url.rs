//! URL 相关工具函数。

/// 将 href 解析为绝对 URL。
///
/// - 已经是 `http://` 或 `https://` 开头的绝对 URL 直接返回。
/// - 相对链接基于 `base` 解析。
/// - 仅接受 `http` / `https` scheme（过滤 `javascript:` `mailto:` `data:` 等）。
pub fn resolve_href(base: &str, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let base_url = url::Url::parse(base).ok()?;
    let joined = base_url.join(href).ok()?;
    if joined.scheme() == "http" || joined.scheme() == "https" {
        Some(joined.to_string())
    } else {
        None
    }
}

/// ND-007-SEC：脱敏 URL 中的凭据（user:pass@host）。
///
/// 错误信息和日志中不应包含 URL 凭据。此函数将 `http://user:pass@host/path`
/// 转换为 `http://***:***@host/path`，避免凭据泄露。
pub fn sanitize_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    if parsed.username().is_empty() {
        return url.to_string();
    }
    let has_password = parsed.password().is_some();
    let _ = parsed.set_username("***");
    if has_password {
        let _ = parsed.set_password(Some("***"));
    } else {
        let _ = parsed.set_password(None);
    }
    parsed.to_string()
}

/// 将 URL 转换为安全的文件名（用于 Markdown 输出等场景）。
///
/// 提取 host + path 组合为文件名，截断至 100 字符，追加 `.md` 后缀。
/// 解析失败时回退为 `page_{counter}.md`。
///
/// ND-004-SEC：过滤 Windows 保留名和路径分隔符，提升跨平台兼容性。
pub fn url_to_filename(url: &str, counter: usize) -> String {
    let parsed = url::Url::parse(url);
    let base = parsed
        .as_ref()
        .map(|u| {
            let host = u.host_str().unwrap_or("page");
            // ND-004-SEC：用 sanitize_filename_component 过滤路径分隔符和 Windows 保留名
            let path = sanitize_filename_component(u.path().trim_matches('/'));
            if path.is_empty() {
                format!("{}_index", host)
            } else {
                format!("{}_{}", host, path)
            }
        })
        .unwrap_or_else(|_| format!("page_{}", counter));
    let truncated: String = base.chars().take(100).collect();
    format!("{}.md", truncated)
}

/// ND-004-SEC：过滤文件名组件，防止路径穿越和 Windows 保留名冲突。
///
/// - 替换 `/` 和 `\` 为 `_`（防止路径分隔符穿越）
/// - Windows 保留名（CON/PRN/AUX/NUL/COM1-9/LPT1-9）加 `wisp_` 前缀
fn sanitize_filename_component(s: &str) -> String {
    let s = s.replace(['/', '\\'], "_");
    // Windows 保留名检查（不区分大小写）
    let upper = s.to_uppercase();
    let is_reserved = matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if is_reserved {
        format!("wisp_{}", s)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === resolve_href ===

    #[test]
    fn absolute_http_url() {
        assert_eq!(
            resolve_href("https://example.com", "https://other.com/p"),
            Some("https://other.com/p".into())
        );
        assert_eq!(
            resolve_href("https://example.com", "http://other.com/p"),
            Some("http://other.com/p".into())
        );
    }

    #[test]
    fn relative_url_resolved() {
        assert_eq!(
            resolve_href("https://example.com/a/", "b"),
            Some("https://example.com/a/b".into())
        );
        assert_eq!(
            resolve_href("https://example.com/page", "/next"),
            Some("https://example.com/next".into())
        );
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(resolve_href("https://example.com", "javascript:void(0)").is_none());
        assert!(resolve_href("https://example.com", "mailto:a@b.com").is_none());
        assert!(resolve_href("https://example.com", "data:text/html,xxx").is_none());
        assert!(resolve_href("https://example.com", "ftp://files.example.com/f").is_none());
    }

    #[test]
    fn invalid_base_returns_none() {
        assert!(resolve_href("not-a-url", "/path").is_none());
    }

    // === url_to_filename ===

    #[test]
    fn basic_url_to_filename() {
        let name = url_to_filename("https://example.com/books/123", 0);
        assert!(name.ends_with(".md"));
        assert!(name.contains("example.com"));
        assert!(name.contains("books_123"));
    }

    #[test]
    fn root_url_to_filename() {
        let name = url_to_filename("https://example.com/", 0);
        assert_eq!(name, "example.com_index.md");
    }

    #[test]
    fn invalid_url_fallback() {
        let name = url_to_filename("not-a-url", 5);
        assert_eq!(name, "page_5.md");
    }

    // === sanitize_url ===

    #[test]
    fn sanitize_url_no_credentials() {
        let url = "https://example.com/path?q=1";
        assert_eq!(sanitize_url(url), url);
    }

    #[test]
    fn sanitize_url_redacts_credentials() {
        let url = "http://user:pass@example.com/path";
        let sanitized = sanitize_url(url);
        assert!(!sanitized.contains("user"), "脱敏后不应包含 username");
        assert!(!sanitized.contains("pass"), "脱敏后不应包含 password");
        assert!(
            sanitized.contains("***:***@"),
            "脱敏后应包含 ***:***@，实际: {}",
            sanitized
        );
        assert!(sanitized.contains("example.com"), "脱敏后应保留 host");
    }

    #[test]
    fn sanitize_url_invalid_url_passthrough() {
        let url = "not a url at all";
        assert_eq!(sanitize_url(url), url);
    }

    #[test]
    fn sanitize_url_only_username() {
        let url = "http://user@example.com/";
        let sanitized = sanitize_url(url);
        assert!(!sanitized.contains("user@"));
        assert!(sanitized.contains("***"));
    }

    #[test]
    fn sanitize_url_password_with_at_fully_redacted() {
        let sanitized = sanitize_url("http://user:p@ss@example.com/path");
        assert!(
            !sanitized.contains("p@ss"),
            "密码含 @ 时不得泄露: {sanitized}"
        );
        assert!(sanitized.contains("***:***@example.com"));
    }

    #[test]
    fn sanitize_url_keeps_query_value_intact() {
        let sanitized = sanitize_url("http://user:pass@example.com/?q=user:pass");
        assert!(
            sanitized.contains("q=user:pass"),
            "query 不应被替换: {sanitized}"
        );
    }

    #[test]
    fn sanitize_url_username_only_keeps_single_star_pair() {
        assert_eq!(
            sanitize_url("http://user@example.com/"),
            "http://***@example.com/"
        );
    }
}
