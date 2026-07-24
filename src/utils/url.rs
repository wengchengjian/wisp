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

/// 将 URL 转换为安全的文件名（用于 Markdown 输出等场景）。
///
/// 提取 host + path 组合为文件名，截断至 100 字符，追加 `.md` 后缀。
/// 解析失败时回退为 `page_{counter}.md`。
pub fn url_to_filename(url: &str, counter: usize) -> String {
    let parsed = url::Url::parse(url);
    let base = parsed
        .as_ref()
        .map(|u| {
            let host = u.host_str().unwrap_or("page");
            let path = u.path().trim_matches('/').replace('/', "_");
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
}
