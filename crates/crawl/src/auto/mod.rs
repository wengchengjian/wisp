//! Auto 模式核心组件：URL 泛化、规则引擎、拦截检测。
//!
//! Auto 模式流程：
//! 1. 匹配用户规则 → 直接用指定模式
//! 2. 匹配自动泛化缓存 → 直接用缓存模式
//! 3. 都没命中 → HTTP 抓取 → 中间件检测是否需要升级

mod blocked;
mod engine;
mod generalize;

#[cfg(test)]
pub(crate) use blocked::blocked_body_reason;
pub use blocked::{blocked_reason, is_blocked_response};
pub use engine::ModeRuleEngine;
pub use generalize::generalize_url;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wisp_fetcher::FetchMode;

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

    #[test]
    fn test_blocked_cf_header_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("Cf-Chl-Bypass".to_string(), "1".to_string());
        assert_eq!(
            blocked_reason(200, b"normal", &headers),
            Some("header:cf-chl")
        );
    }

    #[test]
    fn test_blocked_body_matches_case_insensitive_bytes() {
        assert_eq!(
            blocked_body_reason(b"JUST A MOMENT..."),
            Some("body:just a moment")
        );
        assert_eq!(
            blocked_body_reason(b"CF-Challenge running"),
            Some("body:cf-challenge")
        );
        assert_eq!(blocked_body_reason(b"normal"), None);
        assert_eq!(
            blocked_reason(200, b"<title>JUST A MOMENT...</title>", &HashMap::new()),
            Some("body:just a moment")
        );
    }

    #[test]
    fn test_blocked_body_scan_limited_to_head_window() {
        // 特征出现在 64KB 窗口之后，不应因扫描整页而误判（也不应扫描全 body）。
        let mut body = vec![b'a'; 64 * 1024 + 64];
        body[64 * 1024 + 8..64 * 1024 + 21].copy_from_slice(b"access denied");
        assert_eq!(blocked_reason(200, &body, &HashMap::new()), None);
    }

    // === 从集成测试并入的独有用例 ===

    #[test]
    fn test_generalize_multi_segment_numeric_id() {
        assert_eq!(
            generalize_url("https://shop.com/item/99999/reviews"),
            "/item/\\d+/reviews"
        );
    }

    #[test]
    fn test_generalize_api_version_and_uuid() {
        assert_eq!(
            generalize_url("https://api.io/v1/a1b2c3d4e5f6"),
            "/v\\d+/[a-f0-9-]+"
        );
        assert_eq!(
            generalize_url("https://shop.com/api/v2/items/7"),
            "/api/v\\d+/items/\\d+"
        );
    }

    #[test]
    fn test_multiple_patterns_coexist() {
        let mut engine = ModeRuleEngine::new();
        engine.learn("https://shop.com/products/1", FetchMode::Dynamic);
        engine.learn("https://shop.com/blog/hello-world", FetchMode::Http);

        assert_eq!(
            engine.resolve("https://shop.com/products/5"),
            Some(FetchMode::Dynamic)
        );
        assert_eq!(
            engine.resolve("https://shop.com/blog/hello-world"),
            Some(FetchMode::Http)
        );
    }

    #[test]
    fn test_blocked_404_not_blocked() {
        assert!(!is_blocked_response(404, b"not found", &HashMap::new()));
    }

    #[test]
    fn test_blocked_challenge_platform_not_blocked() {
        assert!(!is_blocked_response(
            200,
            b"challenge-platform/h/b",
            &HashMap::new()
        ));
    }

    #[test]
    fn test_blocked_attention_required() {
        assert!(is_blocked_response(
            200,
            b"Attention Required",
            &HashMap::new()
        ));
    }
}
