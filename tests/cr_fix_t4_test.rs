//! Task 4 回归测试：request_with_session 合并 meta。
//!
//! 验证 `request_with_session` 不再覆盖原有 meta，而是合并 `__sid` 字段。
use wisp::crawl::session::request_with_session;
use wisp::crawl::session::session_id_of;
use wisp::crawl::SpiderRequest;
use serde_json::json;

#[test]
fn test_request_with_session_preserves_meta() {
    // 通过 with_meta 设置原有 meta（如分页信息）
    let req = SpiderRequest::get("https://example.com")
        .with_meta(json!({"page": 2, "category": "books"}));
    let req = request_with_session(req, "stealth");

    // __sid 应被注入
    assert_eq!(req.meta["__sid"], "stealth", "应注入 __sid");
    // 原有 meta 不应丢失
    assert_eq!(req.meta["page"], 2, "原有 meta 不应丢失");
    assert_eq!(req.meta["category"], "books", "原有 meta 不应丢失");
}

#[test]
fn test_request_with_session_still_extractable() {
    let req = SpiderRequest::get("https://example.com")
        .with_meta(json!({"depth": 3}));
    let req = request_with_session(req, "fast");

    // session_id_of 仍能正确提取
    assert_eq!(session_id_of(&req), "fast", "session ID 应可提取");
    // 原有字段保留
    assert_eq!(req.meta["depth"], 3, "原有 meta 不应丢失");
}

#[test]
fn test_request_with_session_null_meta() {
    // meta 默认为 Null，应被规范化为空对象后注入 __sid
    let req = SpiderRequest::get("https://example.com");
    let req = request_with_session(req, "default");

    assert_eq!(session_id_of(&req), "default", "session ID 应可提取");
    assert!(req.meta.is_object(), "meta 应为对象");
    assert_eq!(req.meta["__sid"], "default", "应注入 __sid");
}

#[test]
fn test_request_with_session_overwrites_existing_sid() {
    // 若已有 __sid，应被新值覆盖
    let req = SpiderRequest::get("https://example.com")
        .with_meta(json!({"__sid": "old", "keep": true}));
    let req = request_with_session(req, "new");

    assert_eq!(session_id_of(&req), "new", "__sid 应被更新为新值");
    assert_eq!(req.meta["keep"], true, "其他 meta 字段应保留");
}
