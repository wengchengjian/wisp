//! Text and attribute processing utilities.

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

/// `\s+` 正则：合并连续空白为单个空格。
static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// `<[^>]*>` 正则：剥离 HTML 标签。
static RE_HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());

/// Text processing helper wrapping a string slice.
pub struct Text<'a>(pub &'a str);

impl<'a> Text<'a> {
    /// 创建文本处理器。
    #[must_use]
    pub fn new(s: &'a str) -> Self {
        Self(s)
    }

    /// Collapse all whitespace runs into single spaces and trim.
    pub fn clean(&self) -> String {
        RE_WHITESPACE.replace_all(self.0, " ").trim().to_string()
    }

    /// Extract all matches of a regex pattern.
    pub fn extract_regex(&self, pattern: &str) -> Vec<String> {
        match Regex::new(pattern) {
            Ok(re) => re
                .find_iter(self.0)
                .map(|m| m.as_str().to_string())
                .collect(),
            Err(e) => {
                tracing::warn!("无效正则 '{}': {}", pattern, e);
                Vec::new()
            }
        }
    }

    /// Extract all email addresses.
    #[must_use]
    pub fn extract_emails(&self) -> Vec<String> {
        self.extract_regex(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}")
    }

    /// Extract all URLs (http/https).
    #[must_use]
    pub fn extract_urls(&self) -> Vec<String> {
        self.extract_regex(r#"https?://[^\s<>"']+"#)
    }

    /// Truncate to max characters, appending "..." if truncated.
    #[must_use]
    pub fn truncate(&self, max: usize) -> String {
        if self.0.chars().count() <= max {
            self.0.to_string()
        } else {
            let truncated: String = self.0.chars().take(max).collect();
            format!("{truncated}...")
        }
    }

    /// Strip all HTML tags, returning plain text.
    pub fn strip_tags(&self) -> String {
        RE_HTML_TAG.replace_all(self.0, "").to_string()
    }
}

/// Attribute map helper.
#[derive(Debug, Clone, Default)]
pub struct Attrs(pub HashMap<String, String>);

impl Attrs {
    /// 创建空属性映射。
    #[must_use]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// 获取属性值。
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(std::string::String::as_str)
    }

    /// 插入属性。
    pub fn insert(&mut self, key: String, value: String) {
        self.0.insert(key, value);
    }

    /// 转换为 JSON 对象。
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(&self.0).unwrap_or(Value::Null)
    }

    /// 属性数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean() {
        let t = Text("  hello   world  \n\t foo  ");
        assert_eq!(t.clean(), "hello world foo");
    }

    #[test]
    fn test_extract_emails() {
        let t = Text("contact us at info@example.com or sales@test.org");
        let emails = t.extract_emails();
        assert_eq!(emails.len(), 2);
        assert!(emails.contains(&"info@example.com".to_string()));
    }

    #[test]
    fn test_extract_urls() {
        let t = Text("visit https://example.com and http://test.org/page");
        let urls = t.extract_urls();
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn test_truncate() {
        let t = Text("hello world");
        assert_eq!(t.truncate(5), "hello...");
        assert_eq!(t.truncate(20), "hello world");
    }

    #[test]
    fn test_strip_tags() {
        let t = Text("<p>Hello <b>world</b></p>");
        assert_eq!(t.strip_tags(), "Hello world");
    }

    #[test]
    fn test_attrs() {
        let mut attrs = Attrs::new();
        attrs.insert("href".into(), "https://example.com".into());
        attrs.insert("class".into(), "link".into());
        assert_eq!(attrs.get("href"), Some("https://example.com"));
        assert_eq!(attrs.len(), 2);
        assert!(!attrs.to_json().is_null());
    }
}
