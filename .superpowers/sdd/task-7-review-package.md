04978fa perf(storage): turso 替换 rusqlite，原生 async 无需 spawn_blocking
---STAT---
 Cargo.lock            | 1614 +++++++++++++++++++++++++++++++++++++++++++++----
 Cargo.toml            |    5 +-
 src/bin/wisp.rs       |    2 +-
 src/storage/sqlite.rs |  268 ++++----
 4 files changed, 1630 insertions(+), 259 deletions(-)
---DIFF---
diff --git a/Cargo.toml b/Cargo.toml
index f188a2b..dff7847 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -8,7 +8,7 @@ license = "Apache-2.0"
 [features]
 default = []
 # 启用 SQLite 存储后端（默认禁用，使用 FileStore）
-sqlite = ["dep:rusqlite"]
+sqlite = ["dep:turso"]
 
 [dependencies]
 tokio = { version = "1", features = ["full"] }
@@ -39,7 +39,8 @@ toml = "1"
 regex = "1"
 aho-corasick = "1"
 # SQLite 存储后端（可选，启用 sqlite feature）
-rusqlite = { version = "0.39", features = ["bundled"], optional = true }
+# turso: 原生 async SQLite，内置连接池，无需 spawn_blocking
+turso = { version = "=0.7.0-pre.18", optional = true }
 # checkpoint blob 序列化（bincode 3.0.0 停维，2.x 不兼容 serde_json::Value，锁定 1.3.3 稳定版）
 bincode = "1.3.3"
 # 流式输出（阶段 1 内部用，阶段 3 对外暴露）
diff --git a/src/bin/wisp.rs b/src/bin/wisp.rs
index d3c123e..147a134 100644
--- a/src/bin/wisp.rs
+++ b/src/bin/wisp.rs
@@ -122,7 +122,7 @@ async fn main() -> Result<(), Box<dyn std::error::Error>> {
                     #[cfg(feature = "sqlite")]
                     {
                         if db != ":memory:" && !db.is_empty() {
-                            Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db))?)
+                            Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db)).await?)
                         } else {
                             Arc::new(wisp::FileStore::default())
                         }
diff --git a/src/storage/sqlite.rs b/src/storage/sqlite.rs
index db3a2ca..3b5082b 100644
--- a/src/storage/sqlite.rs
+++ b/src/storage/sqlite.rs
@@ -1,64 +1,83 @@
-//! SQLite 存储后端。单表 KV 结构。
+//! SQLite 存储后端（基于 turso，原生 async）。单表 KV 结构。
 //!
-//! 所有同步 I/O 用 `tokio::task::spawn_blocking` 包装移出 async worker，
-//! 避免阻塞 runtime。`conn` 用 `Arc<Mutex<Connection>>` 让闭包 `'static`。
+//! turso 内部管理连接池，每次操作 `db.connect()` 取独立 Connection，无需手动加锁。
+//! 所有 Store 方法直接 `.await`，不需要 `spawn_blocking`。
 
 use std::path::Path;
-use std::sync::Arc;
 use std::time::Duration;
 
 use async_trait::async_trait;
-use parking_lot::Mutex;
-use rusqlite::{params, Connection};
+use turso::{Builder, Database, Value as TursoValue};
 
-use crate::error::{Result, WispError, StorageError};
+use crate::error::{Result, StorageError, WispError};
 use super::Store;
 
-/// SQLite 存储后端。线程安全（`Arc<parking_lot::Mutex<Connection>>`，无 poison）。
+/// SQLite 存储后端。线程安全（turso `Database` 内部管理连接池）。
 pub struct SqliteStore {
-    conn: Arc<Mutex<Connection>>,
+    db: Database,
 }
 
 impl SqliteStore {
     /// 打开或创建数据库文件。
-    pub fn open(path: &Path) -> Result<Self> {
-        let conn = Connection::open(path)
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-        let store = Self { conn: Arc::new(Mutex::new(conn)) };
-        store.init_schema()?;
+    pub async fn open(path: &Path) -> Result<Self> {
+        let path_str = path.to_str().ok_or_else(|| {
+            WispError::Storage(StorageError::General("invalid path: non-UTF8".into()))
+        })?;
+        let db = Builder::new_local(path_str)
+            .build()
+            .await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso open: {e}"))))?;
+        let store = Self { db };
+        store.init_schema().await?;
         Ok(store)
     }
 
     /// 内存数据库（测试用）。
-    pub fn open_in_memory() -> Result<Self> {
-        let conn = Connection::open_in_memory()
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-        let store = Self { conn: Arc::new(Mutex::new(conn)) };
-        store.init_schema()?;
+    pub async fn open_in_memory() -> Result<Self> {
+        let db = Builder::new_local(":memory:")
+            .build()
+            .await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso open in-memory: {e}"))))?;
+        let store = Self { db };
+        store.init_schema().await?;
         Ok(store)
     }
 
-    fn init_schema(&self) -> Result<()> {
-        let conn = self.conn.lock();
-        conn.execute_batch("PRAGMA journal_mode=WAL;")
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
+    async fn init_schema(&self) -> Result<()> {
+        let conn = self.db.connect()
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
+
+        // PRAGMA journal_mode=WAL 返回一行（新的 mode），不能用 execute_batch，需用 query 消费
+        let mut rows = conn.query("PRAGMA journal_mode=WAL", ())
+            .await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("PRAGMA journal_mode: {e}"))))?;
+        while rows.next().await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("PRAGMA journal_mode next: {e}"))))?.is_some() {}
         conn.execute_batch("PRAGMA synchronous=NORMAL;")
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
+            .await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("PRAGMA synchronous: {e}"))))?;
 
-        // 旧 schema 检测：如果存在旧三表，打印 warning 提示数据已弃用
-        let has_old_table: bool = conn
-            .query_row(
-                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name IN ('element_snapshots', 'crawl_checkpoints', 'response_cache')",
-                [],
-                |row| row.get(0),
-            )
-            .unwrap_or(false);
+        // 旧 schema 检测
+        let mut rows = conn.query(
+            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name IN ('element_snapshots', 'crawl_checkpoints', 'response_cache')",
+            (),
+        ).await
+        .map_err(|e| WispError::Storage(StorageError::General(format!("old schema query: {e}"))))?;
+        let has_old_table = if let Some(row) = rows.next().await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("old schema fetch: {e}"))))? {
+            let val = row.get_value(0)
+                .map_err(|e| WispError::Storage(StorageError::General(format!("old schema get_value: {e}"))))?;
+            matches!(val, TursoValue::Integer(1))
+        } else {
+            false
+        };
         if has_old_table {
             tracing::warn!("检测到旧 schema (element_snapshots/crawl_checkpoints/response_cache 三表)，与新版单表 kv 结构不兼容。旧数据已弃用，建议删除 db 文件重新开始。");
         }
 
         conn.execute_batch(super::migrations::SCHEMA_V1)
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
+            .await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("SCHEMA_V1: {e}"))))?;
         Ok(())
     }
 }
@@ -66,109 +85,81 @@ impl SqliteStore {
 #[async_trait]
 impl Store for SqliteStore {
     async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
-        let conn = Arc::clone(&self.conn);
-        let namespace = namespace.to_string();
-        let key = key.to_string();
-        let value = value.to_vec();
-        tokio::task::spawn_blocking(move || {
-            let conn = conn.lock();
-            let now = chrono::Utc::now().timestamp();
-            conn.execute(
-                "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
-                 VALUES (?1, ?2, ?3, NULL, ?4)",
-                params![namespace, key, value, now],
-            )
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-            Ok(())
-        })
-        .await
-        .map_err(|e| WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}"))))?
+        let conn = self.db.connect()
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
+        let now = chrono::Utc::now().timestamp();
+        conn.execute(
+            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
+             VALUES (?1, ?2, ?3, NULL, ?4)",
+            turso::params![namespace, key, value.to_vec(), now],
+        ).await
+        .map_err(|e| WispError::Storage(StorageError::General(format!("turso set: {e}"))))?;
+        Ok(())
     }
 
     async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
-        let conn = Arc::clone(&self.conn);
-        let namespace = namespace.to_string();
-        let key = key.to_string();
-        tokio::task::spawn_blocking(move || {
-            let conn = conn.lock();
-            let mut stmt = conn.prepare(
-                "SELECT value FROM kv
-                 WHERE namespace = ?1 AND key = ?2
-                   AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
-            )
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-            let mut rows = stmt.query(params![namespace, key])
-                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-            if let Some(row) = rows.next()
-                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))? {
-                let value: Vec<u8> = row.get(0)
-                    .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-                Ok(Some(value))
-            } else {
-                Ok(None)
+        let conn = self.db.connect()
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
+        let mut rows = conn.query(
+            "SELECT value FROM kv \
+             WHERE namespace = ?1 AND key = ?2 \
+               AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
+            turso::params![namespace, key],
+        ).await
+        .map_err(|e| WispError::Storage(StorageError::General(format!("turso get: {e}"))))?;
+        if let Some(row) = rows.next().await
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso get next: {e}"))))? {
+            let val = row.get_value(0)
+                .map_err(|e| WispError::Storage(StorageError::General(format!("turso get_value: {e}"))))?;
+            match val {
+                TursoValue::Blob(b) => Ok(Some(b)),
+                TursoValue::Null => Ok(None),
+                _ => Err(WispError::Storage(StorageError::General("expected blob".into()))),
             }
-        })
-        .await
-        .map_err(|e| WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}"))))?
+        } else {
+            Ok(None)
+        }
     }
 
     async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
-        let conn = Arc::clone(&self.conn);
-        let namespace = namespace.to_string();
-        let key = key.to_string();
-        tokio::task::spawn_blocking(move || {
-            let conn = conn.lock();
-            conn.execute(
-                "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
-                params![namespace, key],
-            )
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-            Ok(())
-        })
-        .await
-        .map_err(|e| WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}"))))?
+        let conn = self.db.connect()
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
+        conn.execute(
+            "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
+            turso::params![namespace, key],
+        ).await
+        .map_err(|e| WispError::Storage(StorageError::General(format!("turso delete: {e}"))))?;
+        Ok(())
     }
 
-    async fn set_with_ttl(
-        &self,
-        namespace: &str,
-        key: &str,
-        value: &[u8],
-        ttl: Option<Duration>,
-    ) -> Result<()> {
-        let conn = Arc::clone(&self.conn);
-        let namespace = namespace.to_string();
-        let key = key.to_string();
-        let value = value.to_vec();
+    async fn set_with_ttl(&self, namespace: &str, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
+        let conn = self.db.connect()
+            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
+        let now = chrono::Utc::now().timestamp();
         let ttl_secs = ttl.map(|d| d.as_secs() as i64);
-        tokio::task::spawn_blocking(move || {
-            let conn = conn.lock();
-            let now = chrono::Utc::now().timestamp();
-            conn.execute(
-                "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
-                 VALUES (?1, ?2, ?3, ?4, ?5)",
-                params![namespace, key, value, ttl_secs, now],
-            )
-            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
-            Ok(())
-        })
-        .await
-        .map_err(|e| WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}"))))?
+        conn.execute(
+            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
+             VALUES (?1, ?2, ?3, ?4, ?5)",
+            turso::params![namespace, key, value.to_vec(), ttl_secs, now],
+        ).await
+        .map_err(|e| WispError::Storage(StorageError::General(format!("turso set_with_ttl: {e}"))))?;
+        Ok(())
     }
 }
 
 #[cfg(test)]
 mod tests {
     use super::*;
+    use std::sync::Arc;
     use std::time::Duration;
 
-    fn make_store() -> SqliteStore {
-        SqliteStore::open_in_memory().unwrap()
+    async fn make_store() -> SqliteStore {
+        SqliteStore::open_in_memory().await.expect("open in-memory sqlite")
     }
 
     #[tokio::test]
     async fn checkpoint_roundtrip() {
-        let store = make_store();
+        let store = make_store().await;
         store.set("checkpoint", "spider1", b"state").await.unwrap();
         assert_eq!(store.get("checkpoint", "spider1").await.unwrap().unwrap(), b"state");
         store.delete("checkpoint", "spider1").await.unwrap();
@@ -177,74 +168,70 @@ mod tests {
 
     #[tokio::test]
     async fn ttl_expiry() {
-        let store = make_store();
+        let store = make_store().await;
         store.set_with_ttl("ns", "k", b"v", Some(Duration::from_secs(1))).await.unwrap();
-        // 手动改 cached_at 让它过期
         {
-            let conn = store.conn.lock();
+            let conn = store.db.connect().unwrap();
             conn.execute(
                 "UPDATE kv SET cached_at = cached_at - 100 WHERE namespace='ns' AND key='k'",
-                [],
-            ).unwrap();
+                (),
+            ).await.unwrap();
         }
         assert!(store.get("ns", "k").await.unwrap().is_none());
     }
 
     #[tokio::test]
     async fn ttl_none_never_expires() {
-        let store = make_store();
+        let store = make_store().await;
         store.set_with_ttl("ns", "k", b"forever", None).await.unwrap();
         assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"forever");
     }
 
     #[tokio::test]
     async fn namespace_isolation() {
-        let store = make_store();
+        let store = make_store().await;
         store.set("ns1", "key", b"a").await.unwrap();
         store.set("ns2", "key", b"b").await.unwrap();
         assert_eq!(store.get("ns1", "key").await.unwrap().unwrap(), b"a");
         assert_eq!(store.get("ns2", "key").await.unwrap().unwrap(), b"b");
     }
 
-    /// 旧 schema 检测：存在旧三表时不应破坏新 kv 表功能。
     #[tokio::test]
     async fn old_schema_detection_does_not_break_new_store() {
         use tempfile::tempdir;
         let dir = tempdir().unwrap();
         let db_path = dir.path().join("test_old_schema.db");
 
-        // 第一次打开：创建新 kv schema 并写入数据
         {
-            let store = SqliteStore::open(&db_path).unwrap();
+            let store = SqliteStore::open(&db_path).await.unwrap();
             store.set("ns", "k", b"v").await.unwrap();
         }
 
-        // 模拟旧 db：直接注入旧三表
         {
-            let conn = rusqlite::Connection::open(&db_path).unwrap();
+            let db = Builder::new_local(db_path.to_str().unwrap())
+                .build()
+                .await
+                .unwrap();
+            let conn = db.connect().unwrap();
             conn.execute_batch(
                 "CREATE TABLE element_snapshots (url TEXT, key TEXT);
                  CREATE TABLE crawl_checkpoints (spider_name TEXT, state BLOB);
                  CREATE TABLE response_cache (url TEXT, method TEXT);",
-            ).unwrap();
+            ).await.unwrap();
         }
 
-        // 重新打开：应检测到旧 schema（打印 warning），但新 kv 表仍可用
-        let store = SqliteStore::open(&db_path).unwrap();
-        // 旧数据仍可读
+        let store = SqliteStore::open(&db_path).await.unwrap();
         assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"v");
-        // 新写入仍可工作
         store.set("ns", "k2", b"v2").await.unwrap();
         assert_eq!(store.get("ns", "k2").await.unwrap().unwrap(), b"v2");
     }
 
-    /// 验证 spawn_blocking 移出 async worker：50 次 set 期间后台 task 应继续推进。
     #[tokio::test]
     async fn test_sqlite_store_async_does_not_block_runtime() {
         use std::sync::atomic::{AtomicU32, Ordering};
         use std::time::{Duration, Instant};
 
-        let store = SqliteStore::open_in_memory().expect("open in-memory sqlite");
+        let store = SqliteStore::open_in_memory().await.expect("open in-memory sqlite");
         let counter = Arc::new(AtomicU32::new(0));
         let c = Arc::clone(&counter);
 
@@ -257,23 +244,14 @@ mod tests {
 
         let start = Instant::now();
         for i in 0..50 {
-            store
-                .set("test_ns", &format!("k{i}"), b"v")
-                .await
-                .expect("set should succeed");
+            store.set("test_ns", &format!("k{i}"), b"v").await.expect("set should succeed");
         }
         let write_elapsed = start.elapsed();
 
         task.await.unwrap();
 
         let counter_val = counter.load(Ordering::SeqCst);
-        assert!(
-            counter_val > 10,
-            "后台 task 应在 SQLite 写入期间继续，实际 counter={counter_val}"
-        );
-        assert!(
-            write_elapsed < Duration::from_secs(5),
-            "50 次 set 应 < 5s，实际 {write_elapsed:?}"
-        );
+        assert!(counter_val > 10, "后台 task 应在 SQLite 写入期间继续，实际 counter={counter_val}");
+        assert!(write_elapsed < Duration::from_secs(5), "50 次 set 应 < 5s，实际 {write_elapsed:?}");
     }
 }
