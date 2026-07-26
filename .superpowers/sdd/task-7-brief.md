# Task 7 (Round 2): Turso 替换 rusqlite

**Files:**
- Modify: `Cargo.toml`（替换依赖）
- Modify: `src/storage/sqlite.rs`（重写 SqliteStore）
- Modify: `src/bin/wisp.rs:125`（`SqliteStore::open` 加 `.await`）

**已确认的调用点（仅此 1 处生产代码 + sqlite.rs 内部测试）：**
- `src/bin/wisp.rs:125` — `Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db))?)` 需改 async + `.await`
- `src/storage/sqlite.rs` 内的 `#[cfg(test)] mod tests` — `make_store` 改 async

**已确认不涉及的位置（不要修改这些）：**
- `src/crawl/runner.rs` — 无 SqliteStore 引用
- `src/mcp/` — 无 SqliteStore 引用
- `tests/` 目录 — 无 SqliteStore::open 直接调用

**Interfaces:**
- Consumes: `turso::{Builder, Database, Value as TursoValue}`、`turso::params!`
- Produces:
  - `SqliteStore::open` 签名从 `pub fn open(path: &Path) -> Result<Self>` 改为 `pub async fn open(path: &Path) -> Result<Self>`
  - `SqliteStore::open_in_memory` 同上改 async

## 已验证的 turso 0.7 API（docs.rs/turso/0.7.0-pre.18）

```rust
use turso::{Builder, Database, Value as TursoValue};

// Builder
let db: Database = Builder::new_local(":memory:").build().await?;
let db: Database = Builder::new_local("/path/to/db.sqlite").build().await?;

// Database（Clone, 内部管理连接池）
let conn: Connection = db.connect()?;  // 同步方法，返回 Result<Connection>

// Connection（async 方法）
conn.execute_batch("PRAGMA journal_mode=WAL;").await?;
conn.execute_batch("CREATE TABLE ...").await?;
let n: u64 = conn.execute("INSERT ...", turso::params![...]).await?;
let mut rows: Rows = conn.query("SELECT ...", turso::params![...]).await?;

// Rows
while let Some(row_result) = rows.next().await? {
    let row: &Row = row_result;
    let val: TursoValue = row.get_value(0)?;
    match val {
        TursoValue::Blob(b: Vec<u8>) => ...,
        TursoValue::Integer(i: i64) => ...,
        TursoValue::Text(s: String) => ...,
        TursoValue::Real(f: f64) => ...,
        TursoValue::Null => ...,
    }
}

// params! 宏接受 owned 或 ref 类型
turso::params![namespace, key, value.to_vec(), now]  // &str, &str, Vec<u8>, i64
turso::params![namespace, key]  // &str, &str
```

## Steps

### Step 1: 修改 Cargo.toml

```toml
# 旧（约 line 30）
rusqlite = { version = "0.31", features = ["bundled"], optional = true }

# 新
turso = { version = "=0.7.0-pre.18", optional = true }

# [features] 部分
# 旧：sqlite = ["dep:rusqlite"]
# 新：sqlite = ["dep:turso"]
```

注意：
- **必须用 `=0.7.0-pre.18`** 精确版本（pre-release 不支持 `^0.7` 自动解析）
- 保留 `sqlite = ["dep:turso"]`，feature gate 名称不变
- 删除 `rusqlite` 依赖

### Step 2: 重写 src/storage/sqlite.rs

**完整重写**，结构如下（参考 plan 文档 880-1164 行的完整代码）：

```rust
//! SQLite 存储后端（基于 turso，原生 async）。单表 KV 结构。
//!
//! turso 内部管理连接池，每次操作 `db.connect()` 取独立 Connection，无需手动加锁。
//! 所有 Store 方法直接 `.await`，不需要 `spawn_blocking`。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use turso::{Builder, Database, Value as TursoValue};

use crate::error::{Result, WispError, StorageError};
use super::Store;

/// SQLite 存储后端。线程安全（turso `Database` 内部管理连接池）。
pub struct SqliteStore {
    db: Database,
}

impl SqliteStore {
    /// 打开或创建数据库文件。
    pub async fn open(path: &Path) -> Result<Self> {
        let path_str = path.to_str().ok_or_else(|| {
            WispError::Storage(StorageError::General("invalid path: non-UTF8".into()))
        })?;
        let db = Builder::new_local(path_str)
            .build()
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso open: {e}"))))?;
        let store = Self { db };
        store.init_schema().await?;
        Ok(store)
    }

    /// 内存数据库（测试用）。
    pub async fn open_in_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso open in-memory: {e}"))))?;
        let store = Self { db };
        store.init_schema().await?;
        Ok(store)
    }

    async fn init_schema(&self) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("PRAGMA journal_mode: {e}"))))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("PRAGMA synchronous: {e}"))))?;

        // 旧 schema 检测
        let mut rows = conn.query(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name IN ('element_snapshots', 'crawl_checkpoints', 'response_cache')",
            (),
        ).await
        .map_err(|e| WispError::Storage(StorageError::General(format!("old schema query: {e}"))))?;
        let has_old_table = if let Some(row) = rows.next().await
            .map_err(|e| WispError::Storage(StorageError::General(format!("old schema fetch: {e}"))))? {
            let val = row.get_value(0)
                .map_err(|e| WispError::Storage(StorageError::General(format!("old schema get_value: {e}"))))?;
            matches!(val, TursoValue::Integer(1))
        } else {
            false
        };
        if has_old_table {
            tracing::warn!("检测到旧 schema (element_snapshots/crawl_checkpoints/response_cache 三表)，与新版单表 kv 结构不兼容。旧数据已弃用，建议删除 db 文件重新开始。");
        }

        conn.execute_batch(super::migrations::SCHEMA_V1)
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("SCHEMA_V1: {e}"))))?;
        Ok(())
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, NULL, ?4)",
            turso::params![namespace, key, value.to_vec(), now],
        ).await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso set: {e}"))))?;
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        let mut rows = conn.query(
            "SELECT value FROM kv \
             WHERE namespace = ?1 AND key = ?2 \
               AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
            turso::params![namespace, key],
        ).await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso get: {e}"))))?;
        if let Some(row) = rows.next().await
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso get next: {e}"))))? {
            let val = row.get_value(0)
                .map_err(|e| WispError::Storage(StorageError::General(format!("turso get_value: {e}"))))?;
            match val {
                TursoValue::Blob(b) => Ok(Some(b)),
                TursoValue::Null => Ok(None),
                _ => Err(WispError::Storage(StorageError::General("expected blob".into()))),
            }
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        conn.execute(
            "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
            turso::params![namespace, key],
        ).await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso delete: {e}"))))?;
        Ok(())
    }

    async fn set_with_ttl(&self, namespace: &str, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        let now = chrono::Utc::now().timestamp();
        let ttl_secs = ttl.map(|d| d.as_secs() as i64);
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            turso::params![namespace, key, value.to_vec(), ttl_secs, now],
        ).await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso set_with_ttl: {e}"))))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn make_store() -> SqliteStore {
        SqliteStore::open_in_memory().await.expect("open in-memory sqlite")
    }

    #[tokio::test]
    async fn checkpoint_roundtrip() {
        let store = make_store().await;
        store.set("checkpoint", "spider1", b"state").await.unwrap();
        assert_eq!(store.get("checkpoint", "spider1").await.unwrap().unwrap(), b"state");
        store.delete("checkpoint", "spider1").await.unwrap();
        assert!(store.get("checkpoint", "spider1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ttl_expiry() {
        let store = make_store().await;
        store.set_with_ttl("ns", "k", b"v", Some(Duration::from_secs(1))).await.unwrap();
        {
            let conn = store.db.connect().unwrap();
            conn.execute(
                "UPDATE kv SET cached_at = cached_at - 100 WHERE namespace='ns' AND key='k'",
                (),
            ).await.unwrap();
        }
        assert!(store.get("ns", "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ttl_none_never_expires() {
        let store = make_store().await;
        store.set_with_ttl("ns", "k", b"forever", None).await.unwrap();
        assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"forever");
    }

    #[tokio::test]
    async fn namespace_isolation() {
        let store = make_store().await;
        store.set("ns1", "key", b"a").await.unwrap();
        store.set("ns2", "key", b"b").await.unwrap();
        assert_eq!(store.get("ns1", "key").await.unwrap().unwrap(), b"a");
        assert_eq!(store.get("ns2", "key").await.unwrap().unwrap(), b"b");
    }

    #[tokio::test]
    async fn old_schema_detection_does_not_break_new_store() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_old_schema.db");

        {
            let store = SqliteStore::open(&db_path).await.unwrap();
            store.set("ns", "k", b"v").await.unwrap();
        }

        {
            let db = Builder::new_local(db_path.to_str().unwrap())
                .build()
                .await
                .unwrap();
            let conn = db.connect().unwrap();
            conn.execute_batch(
                "CREATE TABLE element_snapshots (url TEXT, key TEXT);
                 CREATE TABLE crawl_checkpoints (spider_name TEXT, state BLOB);
                 CREATE TABLE response_cache (url TEXT, method TEXT);",
            ).await.unwrap();
        }

        let store = SqliteStore::open(&db_path).await.unwrap();
        assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"v");
        store.set("ns", "k2", b"v2").await.unwrap();
        assert_eq!(store.get("ns", "k2").await.unwrap().unwrap(), b"v2");
    }

    #[tokio::test]
    async fn test_sqlite_store_async_does_not_block_runtime() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{Duration, Instant};

        let store = SqliteStore::open_in_memory().await.expect("open in-memory sqlite");
        let counter = Arc::new(AtomicU32::new(0));
        let c = Arc::clone(&counter);

        let task = tokio::spawn(async move {
            for _ in 0..100 {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        let start = Instant::now();
        for i in 0..50 {
            store.set("test_ns", &format!("k{i}"), b"v").await.expect("set should succeed");
        }
        let write_elapsed = start.elapsed();

        task.await.unwrap();

        let counter_val = counter.load(Ordering::SeqCst);
        assert!(counter_val > 10, "后台 task 应在 SQLite 写入期间继续，实际 counter={counter_val}");
        assert!(write_elapsed < Duration::from_secs(5), "50 次 set 应 < 5s，实际 {write_elapsed:?}");
    }
}
```

### Step 3: 修改 src/bin/wisp.rs:125

```rust
// 旧
let store: Arc<dyn Store> = Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db))?);

// 新（加 .await）
let store: Arc<dyn Store> = Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db)).await?);
```

注意：确认 `main` 函数是 async（应该是 `#[tokio::main] async fn main()`）。

### Step 4: 编译验证

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

如果出现 `params!` 宏类型不匹配，调整参数类型：
- `&[u8]` → `value.to_vec()`（已在上面的代码中处理）
- `String` → `&str`（直接传 `&str`）
- `i64` → 保持 `i64`

### Step 5: 运行 sqlite 模块测试

Run: `cd /home/weng/wisp && cargo test --lib storage::sqlite --features sqlite`
Expected: 全部 sqlite 测试 PASS

### Step 6: 全量回归

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

### Step 7: Clippy 检查

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features 2>&1 | tail -5`
Expected: 不增加新警告（已有的警告保持不变）

### Step 8: 提交

```bash
cd /home/weng/wisp
git add Cargo.toml Cargo.lock src/storage/sqlite.rs src/bin/wisp.rs
git commit -m "perf(storage): turso 替换 rusqlite，原生 async 无需 spawn_blocking"
```

## 验证

- 编译通过
- sqlite 模块测试 PASS
- 全量测试 PASS（约 435-440 个）
- 无新增 clippy 警告
- 提交信息符合规范

## 注意事项

1. **不要删除 `feature = "sqlite"` gate** — 保持 `sqlite = ["dep:turso"]`
2. **不要修改 src/crawl/runner.rs** — 已确认无 SqliteStore 引用
3. **不要修改 src/mcp/** — 已确认无 SqliteStore 引用
4. **不要修改 tests/ 目录** — 已确认无直接 SqliteStore::open 调用
5. 如果 `super::migrations::SCHEMA_V1` 不存在，搜索 `SCHEMA_V1` 在 src/storage/migrations.rs 中找到定义
6. 如果 `WispError::Storage` 或 `StorageError::General` 不存在，检查 src/error.rs 中的实际变体名
7. 如果 `Arc` 不再使用（旧代码用 `Arc<parking_lot::Mutex<Connection>>`），删除 `use std::sync::Arc;` 如果编译器警告未使用
