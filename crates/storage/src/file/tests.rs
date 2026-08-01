use super::path::sanitize_key;
use super::*;
use crate::Store;
use tempfile::tempdir;

#[test]
fn path_for_sanitizes_namespace() {
    let p = super::path::path_for(std::path::Path::new("root"), "../evil", "key");
    assert_eq!(p, std::path::Path::new("root").join(".._evil").join("key"));
    assert!(
        !p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "namespace 不得产生路径穿越: {}",
        p.display()
    );
}

fn make_store() -> (FileStore, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let store = FileStore::with_dir(dir.path().to_path_buf());
    (store, dir)
}

#[tokio::test]
async fn checkpoint_roundtrip() {
    let (store, _d) = make_store();
    store.set("checkpoint", "spider1", b"state").await.unwrap();
    assert_eq!(
        store.get("checkpoint", "spider1").await.unwrap().unwrap(),
        b"state"
    );
    store.delete("checkpoint", "spider1").await.unwrap();
    assert!(store.get("checkpoint", "spider1").await.unwrap().is_none());
}

#[tokio::test]
async fn ttl_expiry() {
    let (store, _d) = make_store();
    store
        .set_with_ttl("ns", "k", b"v", Some(std::time::Duration::from_millis(1)))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert!(store.get("ns", "k").await.unwrap().is_none());
}

#[tokio::test]
async fn ttl_none_never_expires() {
    let (store, _d) = make_store();
    store
        .set_with_ttl("ns", "k", b"forever", None)
        .await
        .unwrap();
    assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"forever");
}

#[tokio::test]
async fn delete_missing_is_ok() {
    let (store, _d) = make_store();
    store.delete("ns", "nonexistent").await.unwrap();
}

#[tokio::test]
async fn namespace_isolation() {
    let (store, _d) = make_store();
    store.set("ns1", "key", b"a").await.unwrap();
    store.set("ns2", "key", b"b").await.unwrap();
    assert_eq!(store.get("ns1", "key").await.unwrap().unwrap(), b"a");
    assert_eq!(store.get("ns2", "key").await.unwrap().unwrap(), b"b");
}

#[test]
fn sanitize_key_replaces_separators() {
    assert!(sanitize_key("a/b").contains('_'));
    assert!(sanitize_key("a\\b").contains('_'));
    assert!(sanitize_key("a:b").contains('_'));
    // Windows 保留名加前缀
    assert!(sanitize_key("CON").starts_with("wisp_"));
}

/// 验证 spawn_blocking 并发写入：10 个 namespace × 10 次 set 应快速完成。
#[tokio::test]
async fn test_file_store_async_concurrent_writes() {
    use std::sync::Arc;
    use std::time::Instant;

    let tmp = tempdir().unwrap();
    let store = Arc::new(FileStore::with_dir(tmp.path().to_path_buf()));

    let mut handles = Vec::new();
    let start = Instant::now();
    for ns_idx in 0..10 {
        let store = Arc::clone(&store);
        let handle = tokio::spawn(async move {
            let ns = format!("ns_{ns_idx}");
            for i in 0..10 {
                store
                    .set(&ns, &format!("k{i}"), b"v")
                    .await
                    .expect("set should succeed");
            }
        });
        handles.push(handle);
    }
    for handle in handles {
        handle.await.unwrap();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "并发写入应 < 3s，实际 {elapsed:?}"
    );
}
