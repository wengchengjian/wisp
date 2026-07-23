//! Auto 模式测试：URL 泛化、规则引擎、拦截检测、Dynamic 升级中间件。

use wisp::crawl::auto::{generalize_url, ModeRuleEngine, is_blocked_response};
use wisp::FetchMode;
use std::collections::HashMap;

// === URL 泛化测试 ===

#[test]
fn test_generalize_numeric_id() {
    assert_eq!(generalize_url("https://shop.com/products/123"), "/products/\\d+");
    assert_eq!(generalize_url("https://shop.com/page/2"), "/page/\\d+");
    assert_eq!(generalize_url("https://shop.com/item/99999/reviews"), "/item/\\d+/reviews");
}

#[test]
fn test_generalize_uuid() {
    assert_eq!(
        generalize_url("https://shop.com/item/deadbeef-cafe-1234-5678"),
        "/item/[a-f0-9-]+"
    );
    assert_eq!(
        generalize_url("https://api.io/v1/a1b2c3d4e5f6"),
        "/v1/[a-f0-9-]+"
    );
}

#[test]
fn test_generalize_static_path() {
    assert_eq!(generalize_url("https://shop.com/about"), "/about");
    assert_eq!(generalize_url("https://shop.com/products/list"), "/products/list");
    assert_eq!(generalize_url("https://shop.com/"), "/");
}

#[test]
fn test_generalize_mixed() {
    assert_eq!(generalize_url("https://shop.com/user/42/posts"), "/user/\\d+/posts");
    assert_eq!(generalize_url("https://shop.com/api/v2/items/7"), "/api/v2/items/\\d+");
}

// === 规则引擎测试 ===

#[test]
fn test_user_rule_priority() {
    let mut engine = ModeRuleEngine::new();
    engine.add_user_rule(r"/api/.*", FetchMode::Http).unwrap();
    engine.learn("https://shop.com/api/data", FetchMode::Dynamic);

    // 用户规则优先
    assert_eq!(engine.resolve("https://shop.com/api/data"), Some(FetchMode::Http));
}

#[test]
fn test_auto_rule_matches_similar_urls() {
    let mut engine = ModeRuleEngine::new();
    engine.learn("https://shop.com/products/1", FetchMode::Dynamic);

    // 同模板 URL 应命中
    assert_eq!(engine.resolve("https://shop.com/products/2"), Some(FetchMode::Dynamic));
    assert_eq!(engine.resolve("https://shop.com/products/999"), Some(FetchMode::Dynamic));
}

#[test]
fn test_no_rule_returns_none() {
    let engine = ModeRuleEngine::new();
    assert_eq!(engine.resolve("https://shop.com/unknown/page"), None);
}

#[test]
fn test_learn_updates_existing_pattern() {
    let mut engine = ModeRuleEngine::new();
    engine.learn("https://shop.com/products/1", FetchMode::Dynamic);
    engine.learn("https://shop.com/products/2", FetchMode::Stealth);

    // 不新增，更新
    assert_eq!(engine.auto_rule_count(), 1);
    assert_eq!(engine.resolve("https://shop.com/products/3"), Some(FetchMode::Stealth));
}

#[test]
fn test_multiple_patterns_coexist() {
    let mut engine = ModeRuleEngine::new();
    engine.learn("https://shop.com/products/1", FetchMode::Dynamic);
    engine.learn("https://shop.com/blog/hello-world", FetchMode::Http);

    assert_eq!(engine.resolve("https://shop.com/products/5"), Some(FetchMode::Dynamic));
    assert_eq!(engine.resolve("https://shop.com/blog/hello-world"), Some(FetchMode::Http));
}

// === 拦截检测测试 ===

#[test]
fn test_blocked_status_codes() {
    assert!(is_blocked_response(403, b"", &HashMap::new()));
    assert!(is_blocked_response(429, b"", &HashMap::new()));
    assert!(is_blocked_response(503, b"", &HashMap::new()));
    assert!(!is_blocked_response(200, b"ok", &HashMap::new()));
    assert!(!is_blocked_response(404, b"not found", &HashMap::new()));
}

#[test]
fn test_blocked_cf_challenge_in_body() {
    assert!(is_blocked_response(200, b"<title>Just a moment...</title>", &HashMap::new()));
    assert!(is_blocked_response(200, b"<div id='cf-challenge-running'>", &HashMap::new()));
    assert!(is_blocked_response(200, b"challenge-platform/h/b", &HashMap::new()));
    assert!(is_blocked_response(200, b"Attention Required", &HashMap::new()));
    assert!(is_blocked_response(200, b"Access denied", &HashMap::new()));
}

#[test]
fn test_blocked_cf_header() {
    let mut headers = HashMap::new();
    headers.insert("cf-chl-bypass".to_string(), "1".to_string());
    assert!(is_blocked_response(200, b"normal content", &headers));
}

#[test]
fn test_normal_page_not_blocked() {
    let body = b"<html><body><h1>Hello World</h1><p>Content here</p></body></html>";
    assert!(!is_blocked_response(200, body, &HashMap::new()));
}
