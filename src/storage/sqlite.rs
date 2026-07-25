//! SQLite 存储后端。单表 KV 结构。

use std::path::Path;
use std::time::Duration;

use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::error::{Result, WispError, StorageError};
use super::Store;

/// SQLite 存储后端。线程安全（`parking_lot::Mutex<Connection>`，无 poison）。
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// 打开或创建数据库文件。
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let store = Self { conn: Mutex::new(conn) };
        store.init_schema()?;
        Ok(store)
    }

    /// 内存数据库（测试用）。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let store = Self { conn: Mutex::new(conn) };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        conn.execute_batch(super::migrations::SCHEMA_V1)
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }
}

impl Store for SqliteStore {
    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, NULL, ?4)",
            params![namespace, key, value, now],
        )
        .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }

    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT value FROM kv
             WHERE namespace = ?1 AND key = ?2
               AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
        )
        .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let mut rows = stmt.query(params![namespace, key])
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        if let Some(row) = rows.next()
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))? {
            let value: Vec<u8> = row.get(0)
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
            params![namespace, key],
        )
        .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }

    fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp();
        let ttl_secs = ttl.map(|d| d.as_secs() as i64);
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![namespace, key, value, ttl_secs, now],
        )
        .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    #[test]
    fn checkpoint_roundtrip() {
        let store = make_store();
        store.set("checkpoint", "spider1", b"state").unwrap();
        assert_eq!(store.get("checkpoint", "spider1").unwrap().unwrap(), b"state");
        store.delete("checkpoint", "spider1").unwrap();
        assert!(store.get("checkpoint", "spider1").unwrap().is_none());
    }

    #[test]
    fn ttl_expiry() {
        let store = make_store();
        store.set_with_ttl("ns", "k", b"v", Some(Duration::from_secs(1))).unwrap();
        // 手动改 cached_at 让它过期
        store.conn.lock().execute(
            "UPDATE kv SET cached_at = cached_at - 100 WHERE namespace='ns' AND key='k'",
            [],
        ).unwrap();
        assert!(store.get("ns", "k").unwrap().is_none());
    }

    #[test]
    fn ttl_none_never_expires() {
        let store = make_store();
        store.set_with_ttl("ns", "k", b"forever", None).unwrap();
        assert_eq!(store.get("ns", "k").unwrap().unwrap(), b"forever");
    }

    #[test]
    fn namespace_isolation() {
        let store = make_store();
        store.set("ns1", "key", b"a").unwrap();
        store.set("ns2", "key", b"b").unwrap();
        assert_eq!(store.get("ns1", "key").unwrap().unwrap(), b"a");
        assert_eq!(store.get("ns2", "key").unwrap().unwrap(), b"b");
    }
}
