//! SQLite 存储后端。单表 KV 结构。

use std::path::Path;
use std::time::Duration;

use parking_lot::Mutex;
use rusqlite::{params, Connection};

use wisp_core::error::{Result, StorageError, WispError};
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

        // 旧 schema 检测：如果存在旧三表，打印 warning 提示数据已弃用
        let has_old_table: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name IN ('element_snapshots', 'crawl_checkpoints', 'response_cache')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if has_old_table {
            tracing::warn!("检测到旧 schema (element_snapshots/crawl_checkpoints/response_cache 三表)，与新版单表 kv 结构不兼容。旧数据已弃用，建议删除 db 文件重新开始。");
        }

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

    /// 旧 schema 检测：存在旧三表时不应破坏新 kv 表功能。
    #[test]
    fn old_schema_detection_does_not_break_new_store() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_old_schema.db");

        // 第一次打开：创建新 kv schema 并写入数据
        {
            let store = SqliteStore::open(&db_path).unwrap();
            store.set("ns", "k", b"v").unwrap();
        }

        // 模拟旧 db：直接注入旧三表
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE element_snapshots (url TEXT, key TEXT);
                 CREATE TABLE crawl_checkpoints (spider_name TEXT, state BLOB);
                 CREATE TABLE response_cache (url TEXT, method TEXT);",
            ).unwrap();
        }

        // 重新打开：应检测到旧 schema（打印 warning），但新 kv 表仍可用
        let store = SqliteStore::open(&db_path).unwrap();
        // 旧数据仍可读
        assert_eq!(store.get("ns", "k").unwrap().unwrap(), b"v");
        // 新写入仍可工作
        store.set("ns", "k2", b"v2").unwrap();
        assert_eq!(store.get("ns", "k2").unwrap().unwrap(), b"v2");
    }
}
