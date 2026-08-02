//! 统一存储层库测试。

use super::*;
use async_trait::async_trait;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use wisp_core::error::Result;

type MockData = HashMap<(String, String), (Vec<u8>, Option<Instant>)>;

/// 测试用 MockStore：基于 HashMap，支持 TTL 检查。
struct MockStore {
    data: Mutex<MockData>,
}

impl MockStore {
    fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Store for MockStore {
    async fn set(&self, ns: &str, key: &str, value: &[u8]) -> Result<()> {
        self.data
            .lock()
            .insert((ns.into(), key.into()), (value.to_vec(), None));
        Ok(())
    }
    async fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let now = Instant::now();
        let g = self.data.lock();
        if let Some((v, exp)) = g.get(&(ns.into(), key.into())) {
            if let Some(exp) = exp
                && now > *exp
            {
                return Ok(None);
            }
            Ok(Some(v.clone()))
        } else {
            Ok(None)
        }
    }
    async fn delete(&self, ns: &str, key: &str) -> Result<()> {
        self.data.lock().remove(&(ns.into(), key.into()));
        Ok(())
    }
    async fn set_with_ttl(
        &self,
        ns: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let exp = ttl.map(|d| Instant::now() + d);
        self.data
            .lock()
            .insert((ns.into(), key.into()), (value.to_vec(), exp));
        Ok(())
    }
}

fn make_cached(status: u16, body: &[u8], ttl: Option<Duration>) -> CachedResponse {
    CachedResponse {
        status,
        headers: HashMap::new(),
        body: body.to_vec(),
        content_type: "text/html".to_string(),
        cached_at: chrono::Utc::now().timestamp(),
        ttl,
    }
}

#[tokio::test]
async fn checkpoint_roundtrip_via_free_fn() {
    let store = MockStore::new();
    save_checkpoint(&store, "spider1", b"state-bytes")
        .await
        .unwrap();
    let loaded = load_checkpoint(&store, "spider1").await.unwrap().unwrap();
    assert_eq!(loaded, b"state-bytes");
    delete_checkpoint(&store, "spider1").await.unwrap();
    assert!(load_checkpoint(&store, "spider1").await.unwrap().is_none());
}

#[tokio::test]
async fn response_roundtrip_via_free_fn() {
    let store = MockStore::new();
    let resp = make_cached(200, b"<html>hi</html>", Some(Duration::from_secs(3600)));
    save_response(&store, "GET", "https://example.com", &resp)
        .await
        .unwrap();
    let loaded = load_response(&store, "GET", "https://example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.status, 200);
    assert_eq!(loaded.body, b"<html>hi</html>");
    assert_eq!(loaded.content_type, "text/html");
}

#[tokio::test]
async fn response_ttl_expiry() {
    let store = MockStore::new();
    let resp = make_cached(200, b"x", Some(Duration::from_millis(1)));
    save_response(&store, "GET", "https://expired.com", &resp)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        load_response(&store, "GET", "https://expired.com")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn response_no_ttl_never_expires() {
    let store = MockStore::new();
    let resp = make_cached(200, b"forever", None);
    save_response(&store, "GET", "https://forever.com", &resp)
        .await
        .unwrap();
    let loaded = load_response(&store, "GET", "https://forever.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.body, b"forever");
}

#[tokio::test]
async fn method_isolation() {
    let store = MockStore::new();
    save_response(
        &store,
        "GET",
        "https://example.com",
        &make_cached(200, b"get", None),
    )
    .await
    .unwrap();
    save_response(
        &store,
        "POST",
        "https://example.com",
        &make_cached(201, b"post", None),
    )
    .await
    .unwrap();
    assert_eq!(
        load_response(&store, "GET", "https://example.com")
            .await
            .unwrap()
            .unwrap()
            .body,
        b"get"
    );
    assert_eq!(
        load_response(&store, "POST", "https://example.com")
            .await
            .unwrap()
            .unwrap()
            .body,
        b"post"
    );
}

#[tokio::test]
async fn namespace_isolation() {
    let store = MockStore::new();
    // checkpoint 和 element 同名 key 不冲突
    save_checkpoint(&store, "mykey", b"cp").await.unwrap();
    let elem = ElementSnapshotRow {
        tag: "div".into(),
        attrs: serde_json::Value::Null,
        text_preview: "hi".into(),
        ancestor_path: serde_json::Value::Null,
        sibling_tags: serde_json::Value::Null,
        position_in_parent: 0,
        parent_tag: "body".into(),
        parent_attrs: serde_json::Value::Null,
        captured_at: 0,
    };
    save_element(&store, "http://x", "mykey", &elem)
        .await
        .unwrap();
    assert_eq!(
        load_checkpoint(&store, "mykey").await.unwrap().unwrap(),
        b"cp"
    );
    assert!(
        load_element(&store, "http://x", "mykey")
            .await
            .unwrap()
            .is_some()
    );
}

#[test]
fn cached_response_json_snapshot() {
    let resp = CachedResponse {
        status: 200,
        headers: HashMap::from([("content-type".to_string(), "text/html".to_string())]),
        body: b"<h1>Hello</h1>".to_vec(),
        content_type: "text/html; charset=utf-8".to_string(),
        cached_at: 1_700_000_000,
        ttl: Some(Duration::from_secs(3600)),
    };
    insta::assert_snapshot!(
        "cached_response_json",
        serde_json::to_string_pretty(&resp).unwrap()
    );
}

#[test]
fn element_snapshot_row_json_snapshot() {
    let row = ElementSnapshotRow {
        tag: "div".into(),
        attrs: serde_json::json!({ "class": "card" }),
        text_preview: "hello".into(),
        ancestor_path: serde_json::json!(["html", "body", "main"]),
        sibling_tags: serde_json::json!(["section", "div"]),
        position_in_parent: 1,
        parent_tag: "main".into(),
        parent_attrs: serde_json::json!({ "id": "content" }),
        captured_at: 1_700_000_001,
    };
    insta::assert_snapshot!(
        "element_snapshot_row_json",
        serde_json::to_string_pretty(&row).unwrap()
    );
}
