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

#[test]
fn meta_edge_cases_empty_collections_and_bools() {
    // 空对象、空数组、布尔值 — 这些是 JSON 边界用例
    let req = SpiderRequest::get("https://example.com/edge")
        .with_meta(json!({
            "empty_obj": {},
            "empty_arr": [],
            "flag_true": true,
            "flag_false": false,
            "zero": 0,
            "empty_str": ""
        }));

    let bytes = bincode::serialize(&req).expect("serialize");
    let restored: SpiderRequest = bincode::deserialize(&bytes).expect("deserialize");

    assert_eq!(restored.meta, req.meta, "边界用例 meta 必须往返一致");
    assert!(restored.meta["empty_obj"].is_object(), "空对象应保持对象类型");
    assert!(restored.meta["empty_obj"].as_object().unwrap().is_empty(), "空对象应为空");
    assert!(restored.meta["empty_arr"].is_array(), "空数组应保持数组类型");
    assert!(restored.meta["empty_arr"].as_array().unwrap().is_empty(), "空数组应为空");
    assert_eq!(restored.meta["flag_true"], serde_json::Value::Bool(true));
    assert_eq!(restored.meta["flag_false"], serde_json::Value::Bool(false));
    assert_eq!(restored.meta["zero"], 0);
    assert_eq!(restored.meta["empty_str"], "");
}
