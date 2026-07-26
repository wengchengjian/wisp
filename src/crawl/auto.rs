//! Auto 模式核心组件：URL 泛化、规则引擎、拦截检测。
//!
//! Auto 模式流程：
//! 1. 匹配用户规则 → 直接用指定模式
//! 2. 匹配自动泛化缓存 → 直接用缓存模式
//! 3. 都没命中 → HTTP 抓取 → 中间件检测是否需要升级

use crate::error::{Result, WispError};
use crate::fetcher::FetchMode;
use regex::Regex;
use std::collections::HashMap;

// === URL 泛化算法 ===

/// 将具体 URL 泛化为正则模板。
///
/// # 示例
/// - `/products/123` → `/products/\d+`
/// - `/user/deadbeef-cafe-1234/posts` → `/user/[a-f0-9-]+/posts`
/// - `/page/2` → `/page/\d+`
/// - `/about` → `/about`（不变）
/// - `/shuku/0-lastupdate-0-1.html` → `/shuku/\d+-lastupdate-\d+-\d+\.html`
pub fn generalize_url(url: &str) -> String {
    let path = url::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| url.to_string());

    let segments: Vec<String> = path
        .split('/')
        .map(|seg| {
            if seg.is_empty() {
                return String::new();
            }
            // 纯数字 → \d+
            if seg.chars().all(|c| c.is_ascii_digit()) {
                return r"\d+".to_string();
            }
            // UUID/哈希 → [a-f0-9-]+
            if is_uuid_or_hash(seg) {
                return r"[a-f0-9-]+".to_string();
            }
            // 混合段（含数字的字母数字段）：把连续数字序列替换为 \d+
            // 例：0-lastupdate-0-1.html → \d+-lastupdate-\d+-\d+\.html
            if seg.chars().any(|c| c.is_ascii_digit()) {
                return generalize_mixed_segment(seg);
            }
            // 保留字面量（转义正则特殊字符）
            regex::escape(seg)
        })
        .collect();

    segments.join("/")
}

/// 泛化混合段：把所有连续数字序列替换为 `\d+`，其余字符转义。
///
/// 例：`0-lastupdate-0-1.html` → `\d+-lastupdate-\d+-\d+\.html`
///     `product123` → `product\d+`
fn generalize_mixed_segment(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            // 跳过后续连续数字
            while chars.peek().map_or(false, |c| c.is_ascii_digit()) {
                chars.next();
            }
            result.push_str(r"\d+");
        } else {
            // 转义正则特殊字符
            if "\\.+*?()|[]{}^$".contains(c) {
                result.push('\\');
            }
            result.push(c);
        }
    }
    result
}

/// 判断字符串是否像 UUID 或哈希值。
fn is_uuid_or_hash(s: &str) -> bool {
    s.len() >= 8
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.contains(|c: char| c.is_ascii_digit())
}

// === 模式规则引擎 ===

/// URL 模式 → 抓取模式的规则引擎。
///
/// 优先级：用户规则 > 自动学习规则 > None（走 Auto 检测）
pub struct ModeRuleEngine {
    /// 用户定义的规则（优先级最高，按添加顺序匹配）
    user_rules: Vec<(Regex, FetchMode)>,
    /// 自动泛化的缓存规则（运行时学习）
    auto_rules: Vec<(Regex, FetchMode)>,
}

impl ModeRuleEngine {
    /// 创建空的规则引擎。
    pub fn new() -> Self {
        Self {
            user_rules: Vec::new(),
            auto_rules: Vec::new(),
        }
    }

    /// 用户添加规则（优先级最高）。
    pub fn add_user_rule(&mut self, pattern: &str, mode: FetchMode) -> Result<()> {
        let re = Regex::new(pattern).map_err(|e| {
            WispError::Parse(crate::error::ParseError::Html(format!(
                "invalid auto_rule regex '{}': {}",
                pattern, e
            )))
        })?;
        self.user_rules.push((re, mode));
        Ok(())
    }

    /// 自动学习：将 URL 泛化为正则模板后存入。
    ///
    /// 如果相同模板已存在则更新模式。
    pub fn learn(&mut self, url: &str, mode: FetchMode) {
        let pattern = generalize_url(url);
        // 检查是否已有相同模板
        if let Ok(re) = Regex::new(&pattern) {
            // 更新已有规则
            for (existing_re, existing_mode) in &mut self.auto_rules {
                if existing_re.as_str() == re.as_str() {
                    *existing_mode = mode;
                    return;
                }
            }
            // 新增规则
            self.auto_rules.push((re, mode));
        }
    }

    /// 查询 URL 应使用的模式。
    ///
    /// 优先级：用户规则 > 自动规则 > None
    pub fn resolve(&self, url: &str) -> Option<FetchMode> {
        // 提取路径用于匹配
        let path = url::Url::parse(url)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| url.to_string());

        // 用户规则优先
        for (re, mode) in &self.user_rules {
            if re.is_match(&path) || re.is_match(url) {
                return Some(*mode);
            }
        }
        // 自动学习规则
        for (re, mode) in &self.auto_rules {
            if re.is_match(&path) || re.is_match(url) {
                return Some(*mode);
            }
        }
        None
    }

    /// 用户规则数量。
    pub fn user_rule_count(&self) -> usize {
        self.user_rules.len()
    }

    /// 自动规则数量。
    pub fn auto_rule_count(&self) -> usize {
        self.auto_rules.len()
    }
}

impl Default for ModeRuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

// === 拦截检测 ===

/// 检测 HTTP 响应是否被反爬拦截。
///
/// 检测信号：
/// - 状态码 403/429/503
/// - 响应体含 Cloudflare 挑战特征
/// - 响应头含 cf-chl-* 标记
pub fn is_blocked_response(status: u16, body: &[u8], headers: &HashMap<String, String>) -> bool {
    blocked_reason(status, body, headers).is_some()
}

/// 返回拦截原因（用于诊断日志），未拦截则返回 None。
pub fn blocked_reason(status: u16, body: &[u8], headers: &HashMap<String, String>) -> Option<&'static str> {
    // 状态码
    if matches!(status, 403 | 429 | 503) {
        return Some("status_code");
    }
    // CF 特征（即使 200 也可能是挑战页）
    // 注意："challenge-platform" 已移除——它出现在所有 CF 保护页面的正常 HTML 中
    // （<script src="/cdn-cgi/challenge-platform/...">），不是挑战页的标志。
    let text = String::from_utf8_lossy(body).to_lowercase();
    if text.contains("just a moment") {
        return Some("body:just a moment");
    }
    if text.contains("cf-challenge") {
        return Some("body:cf-challenge");
    }
    if text.contains("attention required") {
        return Some("body:attention required");
    }
    if text.contains("access denied") {
        return Some("body:access denied");
    }
    // CF 响应头
    if headers.keys().any(|k| k.starts_with("cf-chl")) {
        return Some("header:cf-chl");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // === URL 泛化测试 ===

    #[test]
    fn test_generalize_numeric_id() {
        assert_eq!(
            generalize_url("https://shop.com/products/123"),
            "/products/\\d+"
        );
        assert_eq!(generalize_url("https://shop.com/page/2"), "/page/\\d+");
    }

    #[test]
    fn test_generalize_uuid() {
        assert_eq!(
            generalize_url("https://shop.com/item/deadbeef-cafe-1234-5678"),
            "/item/[a-f0-9-]+"
        );
    }

    #[test]
    fn test_generalize_static_path() {
        assert_eq!(generalize_url("https://shop.com/about"), "/about");
        assert_eq!(
            generalize_url("https://shop.com/products/list"),
            "/products/list"
        );
    }

    #[test]
    fn test_generalize_mixed() {
        assert_eq!(
            generalize_url("https://shop.com/user/42/posts"),
            "/user/\\d+/posts"
        );
    }

    #[test]
    fn test_generalize_root() {
        assert_eq!(generalize_url("https://shop.com/"), "/");
    }

    #[test]
    fn test_generalize_mixed_segment_pagenum() {
        // banzhu-rs 实际场景：0-lastupdate-0-{N}.html
        assert_eq!(
            generalize_url("https://www.bz555555555.com/shuku/0-lastupdate-0-1.html"),
            "/shuku/\\d+-lastupdate-\\d+-\\d+\\.html"
        );
    }

    #[test]
    fn test_generalize_mixed_segment_productid() {
        assert_eq!(
            generalize_url("https://shop.com/item/product123"),
            "/item/product\\d+"
        );
    }

    #[test]
    fn test_generalize_mixed_segment_multi_pagenum() {
        assert_eq!(
            generalize_url("https://shop.com/page-2-of-10"),
            "/page-\\d+-of-\\d+"
        );
    }

    // === 规则引擎测试 ===

    #[test]
    fn test_user_rule_priority() {
        let mut engine = ModeRuleEngine::new();
        engine.add_user_rule(r"/api/.*", FetchMode::Http).unwrap();
        // 自动规则说 /api/data 需要 Dynamic
        engine.learn("https://shop.com/api/data", FetchMode::Dynamic);

        // 用户规则优先
        assert_eq!(
            engine.resolve("https://shop.com/api/data"),
            Some(FetchMode::Http)
        );
    }

    #[test]
    fn test_auto_rule_matches_similar() {
        let mut engine = ModeRuleEngine::new();
        engine.learn("https://shop.com/products/1", FetchMode::Dynamic);

        // 同模板 URL 应命中
        assert_eq!(
            engine.resolve("https://shop.com/products/2"),
            Some(FetchMode::Dynamic)
        );
        assert_eq!(
            engine.resolve("https://shop.com/products/999"),
            Some(FetchMode::Dynamic)
        );
    }

    #[test]
    fn test_auto_rule_matches_mixed_segment_pagenum() {
        // banzhu-rs 实际场景：0-lastupdate-0-{N}.html
        let mut engine = ModeRuleEngine::new();
        engine.learn(
            "https://www.bz555555555.com/shuku/0-lastupdate-0-1.html",
            FetchMode::Stealth,
        );

        // 不同页码的同模板 URL 应命中
        assert_eq!(
            engine.resolve("https://www.bz555555555.com/shuku/0-lastupdate-0-2.html"),
            Some(FetchMode::Stealth)
        );
        assert_eq!(
            engine.resolve("https://www.bz555555555.com/shuku/0-lastupdate-0-50.html"),
            Some(FetchMode::Stealth)
        );
        // 不同前缀数字也应命中
        assert_eq!(
            engine.resolve("https://www.bz555555555.com/shuku/1-lastupdate-0-1.html"),
            Some(FetchMode::Stealth)
        );
    }

    #[test]
    fn test_no_rule_returns_none() {
        let engine = ModeRuleEngine::new();
        assert_eq!(engine.resolve("https://shop.com/unknown/page"), None);
    }

    #[test]
    fn test_learn_updates_existing() {
        let mut engine = ModeRuleEngine::new();
        engine.learn("https://shop.com/products/1", FetchMode::Dynamic);
        engine.learn("https://shop.com/products/2", FetchMode::Stealth); // 更新

        assert_eq!(engine.auto_rule_count(), 1); // 不新增，更新
        assert_eq!(
            engine.resolve("https://shop.com/products/3"),
            Some(FetchMode::Stealth)
        );
    }

    // === 拦截检测测试 ===

    #[test]
    fn test_blocked_403() {
        assert!(is_blocked_response(403, b"", &HashMap::new()));
        assert!(is_blocked_response(429, b"", &HashMap::new()));
        assert!(is_blocked_response(503, b"", &HashMap::new()));
    }

    #[test]
    fn test_blocked_cf_200() {
        let body = b"<html><title>Just a moment...</title></html>";
        assert!(is_blocked_response(200, body, &HashMap::new()));

        let body2 = b"<div id='cf-challenge-running'></div>";
        assert!(is_blocked_response(200, body2, &HashMap::new()));
    }

    #[test]
    fn test_normal_200_not_blocked() {
        let body = b"<html><body><h1>Hello World</h1></body></html>";
        assert!(!is_blocked_response(200, body, &HashMap::new()));
    }

    #[test]
    fn test_blocked_cf_header() {
        let mut headers = HashMap::new();
        headers.insert("cf-chl-bypass".to_string(), "1".to_string());
        assert!(is_blocked_response(200, b"normal", &headers));
    }
}
