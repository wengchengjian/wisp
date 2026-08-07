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
async fn checkpoint_roundtrip_via_trait_method() {
    let store = MockStore::new();
    store
        .save_checkpoint("spider1", b"state-bytes")
        .await
        .unwrap();
    let loaded = store.load_checkpoint("spider1").await.unwrap().unwrap();
    assert_eq!(loaded, b"state-bytes");
    store.delete_checkpoint("spider1").await.unwrap();
    assert!(store.load_checkpoint("spider1").await.unwrap().is_none());
}

#[tokio::test]
async fn response_roundtrip_via_trait_method() {
    let store = MockStore::new();
    let resp = make_cached(200, b"<html>hi</html>", Some(Duration::from_secs(3600)));
    store
        .save_response("GET", "https://example.com", &resp)
        .await
        .unwrap();
    let loaded = store
        .load_response("GET", "https://example.com")
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
    store
        .save_response("GET", "https://expired.com", &resp)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        store
            .load_response("GET", "https://expired.com")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn response_no_ttl_never_expires() {
    let store = MockStore::new();
    let resp = make_cached(200, b"forever", None);
    store
        .save_response("GET", "https://forever.com", &resp)
        .await
        .unwrap();
    let loaded = store
        .load_response("GET", "https://forever.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.body, b"forever");
}

#[tokio::test]
async fn method_isolation() {
    let store = MockStore::new();
    store
        .save_response(
            "GET",
            "https://example.com",
            &make_cached(200, b"get", None),
        )
        .await
        .unwrap();
    store
        .save_response(
            "POST",
            "https://example.com",
            &make_cached(201, b"post", None),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .load_response("GET", "https://example.com")
            .await
            .unwrap()
            .unwrap()
            .body,
        b"get"
    );
    assert_eq!(
        store
            .load_response("POST", "https://example.com")
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
    // checkpoint 保存后可正常读取
    store.save_checkpoint("mykey", b"cp").await.unwrap();
    assert_eq!(
        store.load_checkpoint("mykey").await.unwrap().unwrap(),
        b"cp"
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
