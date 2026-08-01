use super::*;
use crate::Store;
use std::time::Duration;

fn make_store() -> SqliteStore {
    SqliteStore::open_in_memory().unwrap()
}

#[tokio::test]
async fn checkpoint_roundtrip() {
    let store = make_store();
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
    let store = make_store();
    store
        .set_with_ttl("ns", "k", b"v", Some(Duration::from_secs(1)))
        .await
        .unwrap();
    // 手动改 cached_at 让它过期（先释放锁，避免阻塞 spawn_blocking）
    {
        let conn = store.conn.lock();
        conn.execute(
            "UPDATE kv SET cached_at = cached_at - 100 WHERE namespace='ns' AND key='k'",
            [],
        )
        .unwrap();
    }
    assert!(store.get("ns", "k").await.unwrap().is_none());
}

#[tokio::test]
async fn ttl_none_never_expires() {
    let store = make_store();
    store
        .set_with_ttl("ns", "k", b"forever", None)
        .await
        .unwrap();
    assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"forever");
}

#[tokio::test]
async fn namespace_isolation() {
    let store = make_store();
    store.set("ns1", "key", b"a").await.unwrap();
    store.set("ns2", "key", b"b").await.unwrap();
    assert_eq!(store.get("ns1", "key").await.unwrap().unwrap(), b"a");
    assert_eq!(store.get("ns2", "key").await.unwrap().unwrap(), b"b");
}

/// 旧 schema 检测：存在旧三表时不应破坏新 kv 表功能。
#[tokio::test]
async fn old_schema_detection_does_not_break_new_store() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_old_schema.db");

    // 第一次打开：创建新 kv schema 并写入数据
    {
        let store = SqliteStore::open(&db_path).unwrap();
        store.set("ns", "k", b"v").await.unwrap();
    }

    // 模拟旧 db：直接注入旧三表
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE element_snapshots (url TEXT, key TEXT);
             CREATE TABLE crawl_checkpoints (spider_name TEXT, state BLOB);
             CREATE TABLE response_cache (url TEXT, method TEXT);",
        )
        .unwrap();
    }

    // 重新打开：应检测到旧 schema（打印 warning），但新 kv 表仍可用
    let store = SqliteStore::open(&db_path).unwrap();
    // 旧数据仍可读
    assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"v");
    // 新写入仍可工作
    store.set("ns", "k2", b"v2").await.unwrap();
    assert_eq!(store.get("ns", "k2").await.unwrap().unwrap(), b"v2");
}
