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
    match url::Url::parse(url) {
        Ok(u) => {
            // 无 username，直接返回原 URL
            if u.username().is_empty() {
                return url.to_string();
            }
            // 有凭据，替换为 ***
            let mut s = url.to_string();
            // 替换 username:password@ 为 ***:***@
            if let Some(at_pos) = s.find('@') {
                // 找到 scheme:// 后的位置
                if let Some(scheme_end) = s.find("://") {
                    let creds_start = scheme_end + 3;
                    if at_pos > creds_start {
                        let creds = &s[creds_start..at_pos];
                        s = s.replace(creds, "***:***");
                    }
                }
            }
            s
        }
        Err(_) => url.to_string(),
    }
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
    let s = s.replace('/', "_").replace('\\', "_");
    // Windows 保留名检查（不区分大小写）
    let upper = s.to_uppercase();
    let is_reserved = matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
        | "COM1" | "COM2" | "COM3" | "COM4" | "COM5"
        | "COM6" | "COM7" | "COM8" | "COM9"
        | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5"
        | "LPT6" | "LPT7" | "LPT8" | "LPT9"
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
}
