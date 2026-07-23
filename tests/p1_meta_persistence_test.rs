//! P1-7: SpiderRequest.meta 随 bincode checkpoint 持久化。

use wisp::crawl::SpiderRequest;
use serde_json::json;

#[test]
fn meta_survives_bincode_roundtrip() {
    let req = SpiderRequest::get("https://example.com/page")
        .with_meta(json!({
            "source_page": "https://example.com/list",
            "page_index": 42,
            "tags": ["a", "b"],
            "nested": { "x": 1.5, "y": null }
        }));

    let bytes = bincode::serialize(&req).expect("serialize");
    let restored: SpiderRequest = bincode::deserialize(&bytes).expect("deserialize");

    assert_eq!(restored.url, "https://example.com/page");
    assert_eq!(restored.meta, req.meta, "meta 必须往返保持一致");
    // 抽查嵌套字段
    assert_eq!(restored.meta["page_index"], 42);
    assert_eq!(restored.meta["tags"][1], "b");
    assert_eq!(restored.meta["nested"]["y"], serde_json::Value::Null);
}

#[test]
fn meta_default_null_when_absent() {
    let req = SpiderRequest::get("https://example.com/x");
    let bytes = bincode::serialize(&req).expect("serialize");
    let restored: SpiderRequest = bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(restored.meta, serde_json::Value::Null);
}
