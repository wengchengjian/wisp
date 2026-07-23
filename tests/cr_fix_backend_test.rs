//! Task 5 回归测试：SqliteBackend::delete 必须真正删除行，
//! 使后续 get 返回 None（符合 StorageBackend::get 契约）。
use wisp::storage::backend::{SqliteBackend, StorageBackend};
use wisp::storage::Store;

#[tokio::test]
async fn sqlite_backend_delete_then_get_returns_none() {
    let store = Store::open_in_memory().unwrap();
    let backend = SqliteBackend::new(store);

    backend.set("key1", b"value1").await.unwrap();
    assert_eq!(
        backend.get("key1").await.unwrap(),
        Some(b"value1".to_vec())
    );

    backend.delete("key1").await.unwrap();
    let got = backend.get("key1").await.unwrap();
    assert_eq!(
        got, None,
        "delete 后 get 必须返回 None，实际 {:?}", got
    );
}

#[tokio::test]
async fn sqlite_backend_overwrite_via_set() {
    let store = Store::open_in_memory().unwrap();
    let backend = SqliteBackend::new(store);

    backend.set("k", b"v1").await.unwrap();
    backend.set("k", b"v2").await.unwrap();
    assert_eq!(backend.get("k").await.unwrap(), Some(b"v2".to_vec()));
}

#[tokio::test]
async fn sqlite_backend_delete_missing_key_is_noop() {
    // 删除不存在的键应安全返回 Ok（DELETE 不匹配行不是错误）
    let store = Store::open_in_memory().unwrap();
    let backend = SqliteBackend::new(store);

    backend.delete("never_existed").await.unwrap();
    assert_eq!(backend.get("never_existed").await.unwrap(), None);
}
