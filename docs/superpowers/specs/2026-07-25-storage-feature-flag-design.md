# 存储层 feature 开关与 FileStore 默认实现

- **日期**: 2026-07-25
- **状态**: 设计已确认，待实现
- **作者**: wisp 维护者
- **相关文件**: `src/storage/`, `src/crawl/engine.rs`, `src/crawl/runner.rs`, `src/crawl/middleware/builtin.rs`, `src/parser/adaptive.rs`, `src/mcp/`, `src/bin/wisp.rs`, `Cargo.toml`

---

## 1. 背景与动机

### 1.1 现状

wisp 当前存储层（`src/storage/mod.rs`）的状态：

- `Store` trait 按"业务用途"分方法：`save_checkpoint` / `save_element` / `save_response` 等 9 个方法。
- 已有两个实现：
  - `SqliteStore`（基于 `rusqlite`，三张关系型表：`element_snapshots` / `crawl_checkpoints` / `response_cache`）
  - `MemoryStore`（内部混用 `HashMap` + `moka`，注释标"测试用"）
- `rusqlite` 是**强制依赖**（`Cargo.toml` L37，启用了 `bundled` feature，编译时间长 + 二进制体积大）。
- `EngineBuilder::infra()` 默认 `cache_store = None`、`checkpoint_store = None`，需要用户显式注入。

### 1.2 问题

1. **trait 抽象层级错位**：`Store` 按业务用途定义方法，导致实现者被迫为每个用途写代码。新增 FileStore 时，response cache 落盘语义怪异（缓存不该持久化）。
2. **sqlite 强依赖**：开发者若不需要 sqlite（例如纯内存场景或自定义后端），仍被迫编译 `rusqlite`（含 `libsqlite3` 静态链接）。
3. **MemoryStore 实现冗余**：checkpoint/element 用 `HashMap`、response 用 `moka`，两个数据结构并存。理由是"checkpoint 永不过期 + 不被淘汰"，但 moka 的 per-entry TTL 已能表达"永不过期"（传 `None`），无需额外 `HashMap`。
4. **SqliteStore 伪装成关系型**：审计现有 SQL 调用，**全部是 PK 查询**（`WHERE spider_name=?` / `WHERE url=? AND key=?` / `WHERE url=? AND method=?`），无任何结构化字段查询（`WHERE tag=?` 等）。多列表结构纯属过度设计，attrs/text_preview 等字段实际存的是 JSON 文本，从未被 SQL 解析。
5. **缺少默认持久化后端**：用户必须自己实现 Store 或显式注入，无法开箱即用。

### 1.3 动机

给开发者一个不依赖 sqlite 的选项，作为 wisp 的默认模式。同时把 trait 抽象层级修正到底层 KV 原语，让三种后端（Memory / File / Sqlite）在原语层对等。

---

## 2. 目标与非目标

### 2.1 目标

1. **新增 FileStore**：基于文件系统的持久化存储，作为 wisp 默认实现，不依赖 sqlite。
2. **sqlite 改为可选 feature**：默认禁用，启用时才编译 `rusqlite`。
3. **重构 Store trait 为底层 KV 原语**：仅 `set/get/delete/set_with_ttl` 4 个方法。
4. **业务方法作为自由函数**：`save_checkpoint` / `load_response` 等改为自由函数，接受 `&dyn Store`。
5. **MemoryStore 简化为单 moka**：去掉 `HashMap` 混用。
6. **SqliteStore 简化为单表 KV**：废弃三表结构，改为单张 `kv` 表。
7. **Engine 提供默认 store**：`infra()` 默认注入 MemoryStore（cache）+ FileStore（checkpoint）。
8. **默认存储路径在项目根目录**：`./wisp-data/`（FileStore）/ `./wisp-data/wisp.db`（SqliteStore）。

### 2.2 非目标

- 不实现 Redis 后端（未来可加，trait 设计已兼容）。
- 不实现旧 SqliteStore db 文件的数据迁移（wisp 仍在开发期，无生产数据；旧 db 文件失效，重新爬取即可）。
- 不改变 `Engine`/`EngineBuilder` 的公开 API 形状（除默认值外）。
- 不重构 `ElementSnapshotRow` / `CachedResponse` 的字段定义。
- 不引入异步 trait（现有同步方法已足够快，SQLite/HashMap 操作不阻塞 async runtime）。

---

## 3. 设计概述

### 3.1 架构

```
┌─────────────────────────────────────────────────────────────┐
│ 上层调用方                                                  │
│ Engine / CacheMiddleware / parser::adaptive / mcp::tools    │
│                                                             │
│       调用自由函数：save_checkpoint(&store, ...)             │
│       load_response(&store, ...)                            │
└────────────────────────┬────────────────────────────────────┘
                         │ &dyn Store
                         ▼
┌─────────────────────────────────────────────────────────────┐
│ 业务层（自由函数，src/storage/mod.rs）                       │
│                                                             │
│ save_checkpoint / load_checkpoint / delete_checkpoint       │
│ save_element    / load_element                              │
│ save_response   / load_response   / delete_response          │
│                                                             │
│ 命名空间约定：                                               │
│   "checkpoint" / "element" / "response"                     │
│                                                             │
│ 内部职责：序列化（serde_json）+ key 拼接（"{url}|{key}"）    │
└────────────────────────┬────────────────────────────────────┘
                         │ set/get/delete/set_with_ttl
                         ▼
┌─────────────────────────────────────────────────────────────┐
│ Store trait（4 个底层原语）                                  │
└────┬───────────────────┬───────────────────┬────────────────┘
     │                   │                   │
     ▼                   ▼                   ▼  #[cfg(feature="sqlite")]
┌──────────┐      ┌──────────────┐      ┌──────────────┐
│MemoryStore│     │ FileStore    │      │ SqliteStore  │
│ (单 moka) │      │ (文件系统)   │      │ (单表 KV)    │
│ 默认 cache│      │ 默认 checkpoint│    │              │
└──────────┘      └──────────────┘      └──────────────┘
```

### 3.2 关键决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| trait 抽象层级 | 底层 KV 原语（set/get/delete） | 业务方法做默认实现会让 trait 臃肿；自由函数更符合 Rust 习惯（serde_json、tokio::spawn） |
| sqlite feature 默认值 | 禁用 | 用户需求："不依赖 sqlite 作为默认模式" |
| FileStore 默认路径 | `./wisp-data/` | 用户要求"本地文件默认放在项目根目录" |
| FileStore 文件结构 | 子目录隔离 namespace + 每 entry 一个文件 | checkpoint/element 数据量小，response 走 moka（Engine 层决定） |
| MemoryStore 实现 | 单 moka 实例 | moka 原生支持 per-entry TTL，HashMap 是冗余 |
| SqliteStore 表结构 | 单表 KV (`namespace, key, value, ttl_secs, cached_at`) | 现有 SQL 全是 PK 查询，关系型优势未使用；单表 KV 与其他实现对等 |
| Engine cache_store 默认 | `MemoryStore::default()` | 缓存本就该是内存的，进程退出丢失可接受 |
| Engine checkpoint_store 默认 | `FileStore::default()` | checkpoint 必须持久化，FileStore 默认无依赖 |

---

## 4. 详细设计

### 4.1 Store trait

```rust
// src/storage/mod.rs

use std::time::Duration;
use crate::error::Result;

/// 存储后端 trait。仅提供底层 KV 原语。
///
/// 实现者保证线程安全（`Send + Sync`），内部用 `parking_lot::Mutex` 或 moka 保护。
/// 所有方法同步——SQLite/HashMap/文件 IO 操作足够快，无需 async。
///
/// 业务方法（`save_checkpoint` / `load_response` 等）作为自由函数实现，
/// 调用 `set`/`get`/`delete` 并处理序列化。
pub trait Store: Send + Sync {
    /// 写入一个 entry。
    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()>;

    /// 读取一个 entry。返回 `None` 表示不存在或已过期。
    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>>;

    /// 删除一个 entry。key 不存在不算错误。
    fn delete(&self, namespace: &str, key: &str) -> Result<()>;

    /// 带 TTL 的写入。`ttl = None` 表示永不过期。
    ///
    /// 默认实现忽略 TTL（适用于不支持的存储）。支持 TTL 的实现应覆盖此方法。
    fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        _ttl: Option<Duration>,
    ) -> Result<()> {
        self.set(namespace, key, value)
    }
}
```

### 4.2 业务层自由函数

```rust
// src/storage/mod.rs

use serde::{Serialize, Deserialize};
use crate::error::{WispError, StorageError};

// === 命名空间常量 ===
const NS_CHECKPOINT: &str = "checkpoint";
const NS_ELEMENT: &str = "element";
const NS_RESPONSE: &str = "response";

// === Checkpoint ===

pub fn save_checkpoint(store: &dyn Store, name: &str, state: &[u8]) -> Result<()> {
    store.set(NS_CHECKPOINT, name, state)
}

pub fn load_checkpoint(store: &dyn Store, name: &str) -> Result<Option<Vec<u8>>> {
    store.get(NS_CHECKPOINT, name)
}

pub fn delete_checkpoint(store: &dyn Store, name: &str) -> Result<()> {
    store.delete(NS_CHECKPOINT, name)
}

// === Element Snapshot ===

pub fn save_element(
    store: &dyn Store,
    url: &str,
    key: &str,
    row: &ElementSnapshotRow,
) -> Result<()> {
    let composite = format!("{url}|{key}");
    let bytes = serde_json::to_vec(row)
        .map_err(|e| WispError::Storage(StorageError::General(format!("serialize element: {e}"))))?;
    store.set(NS_ELEMENT, &composite, &bytes)
}

pub fn load_element(
    store: &dyn Store,
    url: &str,
    key: &str,
) -> Result<Option<ElementSnapshotRow>> {
    let composite = format!("{url}|{key}");
    store
        .get(NS_ELEMENT, &composite)?
        .map(|v| serde_json::from_slice(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::General(format!("parse element: {e}"))))
}

// === Response Cache ===

pub fn save_response(
    store: &dyn Store,
    method: &str,
    url: &str,
    resp: &CachedResponse,
) -> Result<()> {
    let composite = format!("{method}|{url}");
    let bytes = serde_json::to_vec(resp)
        .map_err(|e| WispError::Storage(StorageError::General(format!("serialize response: {e}"))))?;
    store.set_with_ttl(NS_RESPONSE, &composite, &bytes, resp.ttl)
}

pub fn load_response(
    store: &dyn Store,
    method: &str,
    url: &str,
) -> Result<Option<CachedResponse>> {
    let composite = format!("{method}|{url}");
    store
        .get(NS_RESPONSE, &composite)?
        .map(|v| serde_json::from_slice(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::General(format!("parse response: {e}"))))
}

pub fn delete_response(store: &dyn Store, method: &str, url: &str) -> Result<()> {
    let composite = format!("{method}|{url}");
    store.delete(NS_RESPONSE, &composite)
}
```

**注意**：旧 trait 的 `save_checkpoint` 第三参数 `saved_at: i64` 被移除。`saved_at` 由调用方自行决定是否写入 state bytes（或可后续作为元数据扩展）。

### 4.3 MemoryStore（src/storage/memory.rs）

```rust
//! 内存存储后端。单 moka 实例，per-entry TTL 原生支持。

use std::time::{Duration, Instant};
use moka::sync::Cache as MokaCache;
use moka::Expiry;
use parking_lot::Mutex;

use crate::error::{Result, WispError, StorageError};
use super::Store;

/// entry 包装：value + 可选过期时间。
#[derive(Clone, Debug)]
struct Entry {
    value: Vec<u8>,
    /// 绝对过期时刻。`None` 表示永不过期。
    expires_at: Option<Instant>,
}

/// per-entry TTL 策略：从 Entry.expires_at 读取。
struct EntryExpiry;

impl Expiry<(String, String), Entry> for EntryExpiry {
    fn expire_after_create(
        &self,
        _key: &(String, String),
        entry: &Entry,
        _now: Instant,
    ) -> Option<Duration> {
        entry.expires_at
            .map(|at| at.saturating_duration_since(_now))
    }
}

/// 内存存储后端。
///
/// 单 moka 实例，capacity 限制总 entry 数（默认 100_000）。
/// TTL 通过 `set_with_ttl` 写入 entry 的 `expires_at` 字段，moka 在过期时自动淘汰。
pub struct MemoryStore {
    inner: MokaCache<(String, String), Entry>,
}

impl MemoryStore {
    pub fn new(capacity: u64) -> Self {
        Self {
            inner: MokaCache::builder()
                .max_capacity(capacity)
                .expire_after(EntryExpiry)
                .build(),
        }
    }
}

impl Default for MemoryStore {
    /// 默认容量 100_000。
    fn default() -> Self {
        Self::new(100_000)
    }
}

impl Store for MemoryStore {
    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let entry = Entry { value: value.to_vec(), expires_at: None };
        self.inner.insert((namespace.to_string(), key.to_string()), entry);
        Ok(())
    }

    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.inner
            .get(&(namespace.to_string(), key.to_string()))
            .map(|e| e.value))
    }

    fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        self.inner.invalidate(&(namespace.to_string(), key.to_string()));
        Ok(())
    }

    fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let expires_at = ttl.map(|d| Instant::now() + d);
        let entry = Entry { value: value.to_vec(), expires_at };
        self.inner.insert((namespace.to_string(), key.to_string()), entry);
        Ok(())
    }
}
```

### 4.4 FileStore（src/storage/file.rs，新增）

```rust
//! 文件系统存储后端。每条 entry 一个文件，子目录隔离 namespace。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::error::{Result, WispError, StorageError};
use super::Store;

/// 将任意 key sanitize 为安全文件名组件。
///
/// 替换文件系统非法字符（`/` `\` `:` `*` `?` `"` `<` `>` `|`）为 `_`，
/// 处理 Windows 保留名（CON/PRN/AUX/NUL/COM1-9/LPT1-9）加 `wisp_` 前缀。
/// 截断至 200 字符防止文件名过长。
fn sanitize_key(key: &str) -> String {
    let s: String = key
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let upper = s.to_uppercase();
    let is_reserved = matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
        | "COM1" | "COM2" | "COM3" | "COM4" | "COM5"
        | "COM6" | "COM7" | "COM8" | "COM9"
        | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5"
        | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    );
    let base = if is_reserved { format!("wisp_{s}") } else { s };
    base.chars().take(200).collect()
}

/// 文件系统存储后端。
///
/// 目录结构：
/// ```text
/// <root>/
/// ├── checkpoint/<sanitized_key>
/// ├── element/<sanitized_key>
/// └── response/<sanitized_key>
/// ```
///
/// TTL 实现：在文件内容前缀附 8 字节 `expires_at`（Unix 秒，big-endian）。
/// `get` 时检查过期，过期则删除文件并返回 `None`。
///
/// 线程安全：单 `parking_lot::Mutex<()>` 保护并发写（文件级互斥简化实现）。
/// 性能：每条 entry 一个文件，适合 checkpoint/element（数据量小）；
///       response cache 不建议使用 FileStore（Engine 默认用 MemoryStore）。
pub struct FileStore {
    root: PathBuf,
    /// 全局写锁（简化实现；可优化为 per-namespace 锁）。
    write_lock: Mutex<()>,
}

impl FileStore {
    /// 自定义根目录。会自动创建。
    pub fn with_dir(root: PathBuf) -> Self {
        let _ = fs::create_dir_all(&root);  // 容忍已存在
        Self { root, write_lock: Mutex::new(()) }
    }

    fn path_for(&self, namespace: &str, key: &str) -> PathBuf {
        let sanitized = sanitize_key(key);
        self.root.join(namespace).join(sanitized)
    }
}

impl Default for FileStore {
    /// 默认根目录 `./wisp-data/`（相对当前工作目录）。
    fn default() -> Self {
        Self::with_dir(PathBuf::from("./wisp-data"))
    }
}

/// 序列化带 TTL 的 entry：`[expires_at: 8 bytes BE i64][value...]`。
/// `expires_at == 0` 表示永不过期。
fn pack_with_ttl(value: &[u8], ttl: Option<Duration>) -> Vec<u8> {
    let expires_at: i64 = match ttl {
        None => 0,
        Some(d) => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH)
                .map(|n| n.as_secs() as i64)
                .unwrap_or(0);
            now + d.as_secs() as i64
        }
    };
    let mut buf = expires_at.to_be_bytes().to_vec();
    buf.extend_from_slice(value);
    buf
}

/// 反序列化，返回 `(value, expired)`。文件损坏返回 `None`。
fn unpack_and_check(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }
    let expires_at = i64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    if expires_at == 0 {
        return Some(data[8..].to_vec());
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|n| n.as_secs() as i64)
        .unwrap_or(0);
    if now > expires_at {
        return None;  // 已过期
    }
    Some(data[8..].to_vec())
}

impl Store for FileStore {
    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let _guard = self.write_lock.lock();
        let path = self.path_for(namespace, key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| WispError::Storage(StorageError::General(format!("mkdir: {e}"))))?;
        }
        let packed = pack_with_ttl(value, None);
        fs::write(&path, packed)
            .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
        Ok(())
    }

    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.path_for(namespace, key);
        let mut file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(WispError::Storage(StorageError::General(format!("open: {e}")))),
        };
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| WispError::Storage(StorageError::General(format!("read: {e}"))))?;
        match unpack_and_check(&buf) {
            Some(v) => Ok(Some(v)),
            None => {
                // 过期或损坏：惰性删除
                let _ = fs::remove_file(&path);
                Ok(None)
            }
        }
    }

    fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let path = self.path_for(namespace, key);
        match fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(WispError::Storage(StorageError::General(format!("delete: {e}")))),
        }
    }

    fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let _guard = self.write_lock.lock();
        let path = self.path_for(namespace, key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| WispError::Storage(StorageError::General(format!("mkdir: {e}"))))?;
        }
        let packed = pack_with_ttl(value, ttl);
        fs::write(&path, packed)
            .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
        Ok(())
    }
}
```

### 4.5 SqliteStore（src/storage/sqlite.rs，重构）

```rust
//! SQLite 存储后端。单表 KV 结构。仅 `feature = "sqlite"` 时编译。

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
```

### 4.6 migrations.rs（重构为单表 KV schema）

```rust
//! SQLite schema migrations for the unified KV store.

/// 单表 KV schema。所有命名空间共享一张表。
pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS kv (
    namespace  TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      BLOB NOT NULL,
    ttl_secs   INTEGER,                -- NULL = 永不过期
    cached_at  INTEGER NOT NULL,       -- Unix 秒，写入时刻
    PRIMARY KEY (namespace, key)
);
CREATE INDEX IF NOT EXISTS idx_kv_namespace ON kv(namespace);
"#;
```

### 4.7 mod.rs 模块组织

```rust
//! src/storage/mod.rs

pub mod migrations;       // schema（仅 sqlite feature 编译）
mod memory;               // MemoryStore
mod file;                 // FileStore

#[cfg(feature = "sqlite")]
mod sqlite;

pub use memory::MemoryStore;
pub use file::FileStore;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

// === Store trait + 自由函数（见 4.1, 4.2）===
// === 公共类型 CachedResponse / ElementSnapshotRow ===
```

### 4.8 Cargo.toml

```toml
[features]
default = []
sqlite = ["dep:rusqlite"]

[dependencies]
# rusqlite 改为可选依赖
rusqlite = { version = "0.40", features = ["bundled"], optional = true }
# 其他依赖不变
```

### 4.9 lib.rs 导出

```rust
pub mod storage;
pub use storage::{
    Store, MemoryStore, FileStore,
    save_checkpoint, load_checkpoint, delete_checkpoint,
    save_element, load_element,
    save_response, load_response, delete_response,
    CachedResponse, ElementSnapshotRow,
};

#[cfg(feature = "sqlite")]
pub use storage::SqliteStore;
```

### 4.10 Engine 默认值

`src/crawl/runner.rs` 的 `EngineBuilder::infra()`：

```rust
pub fn infra() -> EngineBuilder {
    EngineBuilder {
        // ... 其他字段不变
        cache_store: Some(Arc::new(crate::storage::MemoryStore::default())),
        checkpoint_store: Some(Arc::new(crate::storage::FileStore::default())),
        // ...
    }
}
```

注意 `MemoryStore::default()` 实现 `Default` trait（见 4.3），符合 Rust 惯例，允许在 `EngineBuilder::infra()` 中用 `MemoryStore::default()` 或 `Default::default()`。

---

## 5. 调用方迁移

### 5.1 调用点清单

生产代码（9 处）：

| 文件 | 行号 | 旧 | 新 |
|------|------|------|------|
| `src/crawl/runner.rs` | L234 | `store.load_checkpoint(&spider_name)?` | `crate::storage::load_checkpoint(&*store, &spider_name)?` |
| `src/crawl/runner.rs` | L504 | `store.delete_checkpoint(&spider_name)` | `crate::storage::delete_checkpoint(&*store, &spider_name)` |
| `src/crawl/engine.rs` | L528 | `save_checkpoint(store, name, &blob, ts)` | `persist_spider_checkpoint(store, name, sched, stats)`（重命名避免冲突） |
| `src/crawl/engine.rs` | L775 | 测试中 `SqliteStore::open_in_memory()` | `MemoryStore::default()` |
| `src/parser/adaptive.rs` | L277 | `store.save_element(url, key, &row)` | `crate::storage::save_element(&*store, url, key, &row)` |
| `src/parser/adaptive.rs` | L283 | `store.load_element(url, key)` | `crate::storage::load_element(&*store, url, key)` |
| `src/parser/adaptive.rs` | L291 | `store.save_element(url, key, &row)` | `crate::storage::save_element(&*store, url, key, &row)` |
| `src/crawl/middleware/builtin.rs` | L317 | `self.store.load_response(method, &url)` | `crate::storage::load_response(&*self.store, method, &url)` |
| `src/crawl/middleware/builtin.rs` | L349 | `self.store.save_response(method, &url, &cached)` | `crate::storage::save_response(&*self.store, method, &url, &cached)` |

测试代码（24 处，全在 `src/storage/mod.rs` 内部测试模块）：从 `store.save_xxx(...)` 改为 `save_xxx(&store, ...)`。

### 5.2 命名冲突处理

`src/crawl/engine.rs:502` 现有自由函数 `save_checkpoint(store, spider_name, sched, stats)` 与新 `storage::save_checkpoint(store, name, bytes)` 同名。

**处理**：现有 `engine::save_checkpoint` 重命名为 `persist_spider_checkpoint`，内部调用 `storage::save_checkpoint` 写底层 bytes：

```rust
async fn persist_spider_checkpoint(
    store: &dyn crate::storage::Store,
    spider_name: &str,
    sched: &scheduler::Scheduler,
    stats: &Arc<SpiderStats>,
) -> Result<()> {
    let blob = serialize_state(sched, stats).await?;
    crate::storage::save_checkpoint(store, spider_name, &blob)
}
```

### 5.3 bin/wisp.rs 改动

现有 CLI 参数 `--db` 用于指定 sqlite 文件路径：

```rust
// 旧
let store: Arc<dyn Store> = if in_memory {
    Arc::new(wisp::SqliteStore::open_in_memory()?)
} else {
    Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db))?)
};
```

新行为：
- 默认用 `FileStore::default()`（无需 `--db` 参数）
- `--db <path>` 启用时（需启用 sqlite feature）用 SqliteStore
- 用 `#[cfg(feature = "sqlite")]` 条件编译 sqlite 分支

### 5.4 mcp/ 测试改动

`src/mcp/tools.rs:276` 和 `src/mcp/mod.rs:265`：测试中 `SqliteStore::open_in_memory()` 改为 `MemoryStore::default()`（无需 sqlite 依赖）。

---

## 6. 测试策略

### 6.1 单元测试

每个 Store 实现独立测试，覆盖 trait 契约：

```rust
// 测试矩阵（每个 Store 跑一遍）
fn test_store_roundtrip(store: &dyn Store) { ... }
fn test_store_delete_missing(store: &dyn Store) { ... }
fn test_store_ttl_expiry(store: &dyn Store) { ... }
fn test_store_ttl_none_never_expires(store: &dyn Store) { ... }
fn test_store_namespace_isolation(store: &dyn Store) { ... }
```

- `MemoryStore` 测试：在 `src/storage/memory.rs` 内 `#[cfg(test)] mod tests`
- `FileStore` 测试：在 `src/storage/file.rs` 内 `#[cfg(test)] mod tests`，用 `tempfile::tempdir()` 隔离
- `SqliteStore` 测试：在 `src/storage/sqlite.rs` 内 `#[cfg(test)] mod tests`，用 `open_in_memory`

### 6.2 业务层自由函数测试

`src/storage/mod.rs` 测试：mock 一个 `MockStore`（基于 HashMap），验证自由函数的序列化/反序列化逻辑、命名空间使用、key 拼接。

### 6.3 集成测试

- 现有 `tests/crawl_checkpoint_test.rs` 改用 `FileStore` 跑端到端断点续爬
- 现有 `tests/crawl_cache_real_test.rs` 用 `MemoryStore`（默认）和 `SqliteStore`（feature 启用时）分别跑
- 新增 `tests/file_store_e2e.rs`：FileStore 在临时目录跑 checkpoint + element 读写

### 6.4 feature 编译矩阵

CI 矩阵：
- `cargo build`（默认，无 sqlite）：必须通过
- `cargo build --features sqlite`：必须通过
- `cargo test`（默认）：MemoryStore + FileStore 测试
- `cargo test --features sqlite`：增加 SqliteStore 测试

---

## 7. 破坏性影响

### 7.1 上游项目（banzhu-rs）

- `Cargo.toml`：若使用 `SqliteStore`，需加 `features = ["sqlite"]`；若不用，零改动
- `src/scheduler.rs`：调用 `EngineBuilder::infra()` 零改动，自动获得 MemoryStore + FileStore 默认值
- `tests/wisp_engine_integration.rs`：零改动

### 7.2 旧 SqliteStore db 文件

旧 `.db` 文件（三表结构）与新 schema（单表 KV）不兼容。wisp 启动时检测旧 schema 不报错（`CREATE TABLE IF NOT EXISTS`），但旧表数据不会迁移到 `kv` 表。

**建议**：在 `SqliteStore::init_schema` 中检测旧表是否存在，存在则打印 warning 日志提示用户旧数据已弃用。

### 7.3 API 变化

- `Store` trait 方法签名变化（业务方法从 trait 移除）
- `SqliteStore` 改为 `#[cfg(feature = "sqlite")]` 条件编译
- `EngineBuilder::infra()` 默认值从 `None` 改为 `Some(MemoryStore)` + `Some(FileStore)`
- `MemoryStore::new(max_response_entries)` 签名改为 `new(capacity)`（语义不变，参数名变化）

---

## 8. 风险与缓解

| 风险 | 缓解 |
|------|------|
| FileStore 全局写锁影响性能 | 短期可接受（checkpoint/element 频率低）；长期可优化为 per-namespace 锁或 per-file 锁 |
| FileStore 大量小文件影响文件系统性能 | response cache 默认走 MemoryStore 不落盘；FileStore 主要承担 checkpoint/element（数据量小） |
| moka 容量上限导致 MemoryStore 淘汰 checkpoint | 容量设大（100_000），且 checkpoint 调用 `set`（TTL=None 永不过期），moka 的 LRU 不会主动淘汰近期访问数据 |
| 旧 SqliteStore db 文件不兼容 | wisp 开发期无生产数据；启动时检测旧表打印 warning |
| `bin/wisp.rs` CLI 行为变化 | 默认不传 `--db` 时用 FileStore；`--db` 需启用 sqlite feature，否则编译错误 |

---

## 9. 不在范围内

- Redis 后端（trait 设计已兼容，未来可加 `redis` feature + `RedisStore`）
- 旧 SqliteStore db 文件数据迁移工具
- Store trait 异步化（同步方法足够快）
- `ElementSnapshotRow` / `CachedResponse` 字段重构
- `Engine`/`EngineBuilder` 公开 API 形状变化（除默认值外）
```
