## Task 1: 完整重构 storage 模块 + 迁移调用方

**为什么这是一个大任务**：trait 缩小（删除 9 个业务方法）后，所有调用方代码（`store.save_checkpoint(...)` 等 9 处）会立即编译失败，必须同时迁移到自由函数才能让 crate 编译通过。Store 实现也必须同步重写（旧 impl 块因 trait 缩小而失效）。这是一个原子性变更，无法再细分为可独立 commit 的子任务。

**Files:**
- Create: `src/storage/memory.rs`
- Create: `src/storage/file.rs`
- Create: `src/storage/sqlite.rs`
- Rewrite: `src/storage/mod.rs`
- Rewrite: `src/storage/migrations.rs`
- Modify: `src/crawl/runner.rs:234,504`
- Modify: `src/crawl/engine.rs:501-535,775`
- Modify: `src/parser/adaptive.rs:277,283,291`
- Modify: `src/crawl/middleware/builtin.rs:317,349`
- Test: `src/storage/mod.rs` 内部 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `Store` trait（4 原语：set/get/delete/set_with_ttl）、9 个自由函数（save_checkpoint/load_checkpoint/delete_checkpoint/save_element/load_element/save_response/load_response/delete_response）、`MemoryStore`、`FileStore`、`SqliteStore`

- [ ] **Step 1: 重写 src/storage/mod.rs（trait + 自由函数 + 类型 + MockStore 测试）**

完整代码参见 spec 4.1 (trait) + 4.2 (自由函数) + 现有 mod.rs L66-109 (CachedResponse/ElementSnapshotRow 类型保留)。模块组织按 spec 4.7：

```rust
//! 统一存储层：可插拔的持久化后端 trait + Memory/File/SQLite 实现。
//!
//! 三类用途（通过自由函数实现，trait 仅提供底层 KV 原语）：
//! - Checkpoint（断点续爬）：`save_checkpoint` / `load_checkpoint` / `delete_checkpoint`
//! - Element Snapshot（自适应定位）：`save_element` / `load_element`
//! - Response Cache（HTTP 响应缓存，带 per-entry TTL）：`save_response` / `load_response` / `delete_response`

pub mod migrations;

mod memory;
mod file;
mod sqlite;

pub use memory::MemoryStore;
pub use file::FileStore;
pub use sqlite::SqliteStore;

// 注：Task 3 会把 SqliteStore 和 migrations 改为 #[cfg(feature = "sqlite")] 条件编译

use std::time::Duration;
use serde::{Deserialize, Serialize};
use crate::error::{Result, WispError, StorageError};

// ============================================================================
// Store trait：仅底层 KV 原语
// ============================================================================

/// 存储后端 trait。仅提供底层 KV 原语。
///
/// 实现者保证线程安全（`Send + Sync`）。所有方法同步——
/// SQLite/HashMap/文件 IO 操作足够快，无需 async。
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
    /// 默认实现忽略 TTL（适用于不支持的存储）。
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

// ============================================================================
// 公共数据类型（保留不变）
// ============================================================================

/// 可缓存的响应数据（`Response` 的可序列化子集）。
///
/// 不含 `request` 字段——命中时由 `CacheMiddleware` 用当前请求重建完整 `Response`。
/// `cached_at` + `ttl` 配对决定过期时间。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Vec<u8>,
    pub content_type: String,
    /// 缓存时刻（Unix 秒）。
    pub cached_at: i64,
    /// 有效期。`None` 表示永不过期。
    pub ttl: Option<Duration>,
}

impl CachedResponse {
    /// 是否已过期（基于 `cached_at` + `ttl` 与当前时间比较）。
    pub fn is_expired(&self) -> bool {
        match self.ttl {
            Some(ttl) => {
                let now = chrono::Utc::now().timestamp();
                now > self.cached_at + ttl.as_secs() as i64
            }
            None => false,
        }
    }
}

/// Element snapshot 行（存储层不感知 `parser::Node`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSnapshotRow {
    pub tag: String,
    pub attrs: serde_json::Value,
    pub text_preview: String,
    pub ancestor_path: serde_json::Value,
    pub sibling_tags: serde_json::Value,
    pub position_in_parent: i64,
    pub parent_tag: String,
    pub parent_attrs: serde_json::Value,
    pub captured_at: i64,
}

// ============================================================================
// 业务层自由函数
// ============================================================================

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

// ============================================================================
// 测试：自由函数 + MockStore
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use parking_lot::Mutex;

    /// 测试用 MockStore：基于 HashMap，支持 TTL 检查。
    struct MockStore {
        data: Mutex<HashMap<(String, String), (Vec<u8>, Option<Instant>)>>,
    }

    impl MockStore {
        fn new() -> Self {
            Self { data: Mutex::new(HashMap::new()) }
        }
    }

    use std::time::Instant;

    impl Store for MockStore {
        fn set(&self, ns: &str, key: &str, value: &[u8]) -> Result<()> {
            self.data.lock().insert((ns.into(), key.into()), (value.to_vec(), None));
            Ok(())
        }
        fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>> {
            let now = Instant::now();
            let g = self.data.lock();
            if let Some((v, exp)) = g.get(&(ns.into(), key.into())) {
                if let Some(exp) = exp {
                    if now > *exp { return Ok(None); }
                }
                Ok(Some(v.clone()))
            } else { Ok(None) }
        }
        fn delete(&self, ns: &str, key: &str) -> Result<()> {
            self.data.lock().remove(&(ns.into(), key.into()));
            Ok(())
        }
        fn set_with_ttl(&self, ns: &str, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
            let exp = ttl.map(|d| Instant::now() + d);
            self.data.lock().insert((ns.into(), key.into()), (value.to_vec(), exp));
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

    #[test]
    fn checkpoint_roundtrip_via_free_fn() {
        let store = MockStore::new();
        save_checkpoint(&store, "spider1", b"state-bytes").unwrap();
        let loaded = load_checkpoint(&store, "spider1").unwrap().unwrap();
        assert_eq!(loaded, b"state-bytes");
        delete_checkpoint(&store, "spider1").unwrap();
        assert!(load_checkpoint(&store, "spider1").unwrap().is_none());
    }

    #[test]
    fn response_roundtrip_via_free_fn() {
        let store = MockStore::new();
        let resp = make_cached(200, b"<html>hi</html>", Some(Duration::from_secs(3600)));
        save_response(&store, "GET", "https://example.com", &resp).unwrap();
        let loaded = load_response(&store, "GET", "https://example.com").unwrap().unwrap();
        assert_eq!(loaded.status, 200);
        assert_eq!(loaded.body, b"<html>hi</html>");
        assert_eq!(loaded.content_type, "text/html");
    }

    #[test]
    fn response_ttl_expiry() {
        let store = MockStore::new();
        let resp = make_cached(200, b"x", Some(Duration::from_millis(1)));
        save_response(&store, "GET", "https://expired.com", &resp).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(load_response(&store, "GET", "https://expired.com").unwrap().is_none());
    }

    #[test]
    fn response_no_ttl_never_expires() {
        let store = MockStore::new();
        let resp = make_cached(200, b"forever", None);
        save_response(&store, "GET", "https://forever.com", &resp).unwrap();
        let loaded = load_response(&store, "GET", "https://forever.com").unwrap().unwrap();
        assert_eq!(loaded.body, b"forever");
    }

    #[test]
    fn method_isolation() {
        let store = MockStore::new();
        save_response(&store, "GET", "https://example.com", &make_cached(200, b"get", None)).unwrap();
        save_response(&store, "POST", "https://example.com", &make_cached(201, b"post", None)).unwrap();
        assert_eq!(load_response(&store, "GET", "https://example.com").unwrap().unwrap().body, b"get");
        assert_eq!(load_response(&store, "POST", "https://example.com").unwrap().unwrap().body, b"post");
    }

    #[test]
    fn namespace_isolation() {
        let store = MockStore::new();
        // checkpoint 和 element 同名 key 不冲突
        save_checkpoint(&store, "mykey", b"cp").unwrap();
        let elem = ElementSnapshotRow {
            tag: "div".into(), attrs: serde_json::Value::Null,
            text_preview: "hi".into(), ancestor_path: serde_json::Value::Null,
            sibling_tags: serde_json::Value::Null, position_in_parent: 0,
            parent_tag: "body".into(), parent_attrs: serde_json::Value::Null,
            captured_at: 0,
        };
        save_element(&store, "http://x", "mykey", &elem).unwrap();
        assert_eq!(load_checkpoint(&store, "mykey").unwrap().unwrap(), b"cp");
        assert!(load_element(&store, "http://x", "mykey").unwrap().is_some());
    }
}
```

- [ ] **Step 2: 创建 src/storage/memory.rs（单 moka 实现）**

完整代码参见 spec 4.3。要点：
- `Entry` 结构体包装 `value: Vec<u8>` + `expires_at: Option<Instant>`
- `EntryExpiry` 实现 `moka::Expiry` trait，从 entry 读取 TTL
- `MemoryStore` 内部仅一个 `MokaCache<(String, String), Entry>`
- 实现 `Default` trait（容量 100_000）
- 实现 `Store` trait 的 4 个原语

- [ ] **Step 3: 创建 src/storage/file.rs（FileStore 文件系统实现）**

完整代码参见 spec 4.4。要点：
- `sanitize_key(key: &str) -> String`：替换 9 个非法字符（`/` `\` `:` `*` `?` `"` `<` `>` `|`）为 `_`，处理 Windows 保留名，截断 200 字符
- `pack_with_ttl(value, ttl) -> Vec<u8>`：前缀 8 字节 `expires_at`（BE i64，0 = 永不过期）
- `unpack_and_check(data) -> Option<Vec<u8>>`：检查过期，过期返回 `None`
- `FileStore` 内部 `root: PathBuf` + `write_lock: Mutex<()>`
- 实现 `Default` trait（默认 `./wisp-data/`）
- 实现 `Store` trait 的 4 个原语（get 时惰性删除过期文件）
- 测试用 `tempfile::tempdir()` 隔离

测试代码：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_store() -> (FileStore, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let store = FileStore::with_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn checkpoint_roundtrip() {
        let (store, _d) = make_store();
        store.set("checkpoint", "spider1", b"state").unwrap();
        assert_eq!(store.get("checkpoint", "spider1").unwrap().unwrap(), b"state");
        store.delete("checkpoint", "spider1").unwrap();
        assert!(store.get("checkpoint", "spider1").unwrap().is_none());
    }

    #[test]
    fn ttl_expiry() {
        let (store, _d) = make_store();
        store.set_with_ttl("ns", "k", b"v", Some(Duration::from_millis(1))).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(store.get("ns", "k").unwrap().is_none());
    }

    #[test]
    fn ttl_none_never_expires() {
        let (store, _d) = make_store();
        store.set_with_ttl("ns", "k", b"forever", None).unwrap();
        assert_eq!(store.get("ns", "k").unwrap().unwrap(), b"forever");
    }

    #[test]
    fn delete_missing_is_ok() {
        let (store, _d) = make_store();
        store.delete("ns", "nonexistent").unwrap();
    }

    #[test]
    fn namespace_isolation() {
        let (store, _d) = make_store();
        store.set("ns1", "key", b"a").unwrap();
        store.set("ns2", "key", b"b").unwrap();
        assert_eq!(store.get("ns1", "key").unwrap().unwrap(), b"a");
        assert_eq!(store.get("ns2", "key").unwrap().unwrap(), b"b");
    }

    #[test]
    fn sanitize_key_replaces_separators() {
        assert!(sanitize_key("a/b").contains('_'));
        assert!(sanitize_key("a\\b").contains('_'));
        assert!(sanitize_key("a:b").contains('_'));
        // Windows 保留名加前缀
        assert!(sanitize_key("CON").starts_with("wisp_"));
    }
}
```

- [ ] **Step 4: 重写 src/storage/sqlite.rs（单表 KV）**

完整代码参见 spec 4.5。要点：
- 整文件用 `#[cfg(feature = "sqlite")]` 包裹（在 mod.rs 中通过 `#[cfg(feature = "sqlite")] mod sqlite;` 控制，sqlite.rs 本身不需要再加）
- 单表 `kv` (namespace, key, value, ttl_secs, cached_at)
- `get` 用 SQL 过滤过期：`cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER)`
- 实现 4 个原语

测试代码（参考 spec 6.1 测试矩阵）：

```rust
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
```

- [ ] **Step 5: 重写 src/storage/migrations.rs（单表 KV schema）**

完整代码参见 spec 4.6。整个文件改为单表 `kv` schema + 索引。注意：旧三表 schema 完全删除。

- [ ] **Step 6: 迁移调用方 src/crawl/runner.rs（2 处）**

L234:
```rust
// 旧
if let Some(blob) = store.load_checkpoint(&spider_name)? {
// 新
if let Some(blob) = crate::storage::load_checkpoint(&**store, &spider_name)? {
```

L504:
```rust
// 旧
if let Err(e) = store.delete_checkpoint(&spider_name) {
// 新
if let Err(e) = crate::storage::delete_checkpoint(&**store, &spider_name) {
```

注意：`store` 类型是 `Option<Arc<dyn Store>>`，解构后是 `&Arc<dyn Store>`，需要 `&**store` 解两层引用得到 `&dyn Store`。

- [ ] **Step 7: 重命名 engine::save_checkpoint → persist_spider_checkpoint（src/crawl/engine.rs L501-535）**

重命名函数 + 内部调用改为自由函数：

```rust
/// Checkpoint 保存（从 sched + stats 序列化状态，调用底层 save_checkpoint）。
///
/// ND-003-ERR：返回 `Result<()>` 让调用方感知失败。
pub(crate) async fn persist_spider_checkpoint(
    store: &dyn crate::storage::Store,
    spider_name: &str,
    sched: &scheduler::Scheduler,
    stats: &Arc<SpiderStats>,
) -> Result<()> {
    let pending = sched.pending_urls().await;
    let seen = sched.seen_urls().await;
    let snapshot = snapshot_stats_for(stats, HashMap::new(), stats.start);
    let state = CrawlState {
        spider_name: spider_name.to_string(),
        pending_urls: pending,
        seen_urls: seen,
        items_scraped: snapshot.items_scraped,
        pages_crawled: snapshot.pages_crawled,
        errors: snapshot.errors,
        duration_ms: snapshot.duration.as_millis(),
        saved_at: chrono::Utc::now(),
    };
    let blob = bincode::serialize(&state).map_err(|e| {
        crate::error::WispError::Storage(crate::error::StorageError::General(format!(
            "checkpoint 序列化失败: {e}"
        )))
    })?;
    crate::storage::save_checkpoint(store, spider_name, &blob).map_err(|e| {
        crate::error::WispError::Storage(crate::error::StorageError::General(format!(
            "checkpoint 保存失败: {e}"
        )))
    })?;
    Ok(())
}
```

同步更新调用方（在 runner.rs 或 engine.rs 内部调用 `save_checkpoint(store, ...)` 的地方，改为 `persist_spider_checkpoint(store, ...)`）。

- [ ] **Step 8: 修复 src/crawl/engine.rs L775 测试**

```rust
// 旧
let store = crate::storage::SqliteStore::open_in_memory().expect("open in-memory store");
// 新
let store = crate::storage::MemoryStore::default();
```

- [ ] **Step 9: 迁移调用方 src/parser/adaptive.rs（3 处）**

L277, L291:
```rust
// 旧
let _ = store.save_element(url, key, &snap.to_row(now));
// 新
let _ = crate::storage::save_element(&*store, url, key, &snap.to_row(now));
```

L283:
```rust
// 旧
let saved_row = store.load_element(url, key).ok().flatten()?;
// 新
let saved_row = crate::storage::load_element(&*store, url, key).ok().flatten()?;
```

- [ ] **Step 10: 迁移调用方 src/crawl/middleware/builtin.rs（2 处）**

L317:
```rust
// 旧
match self.store.load_response(method_str, &req.url) {
// 新
match crate::storage::load_response(&*self.store, method_str, &req.url) {
```

L349:
```rust
// 旧
if let Err(e) = self.store.save_response(method_str, &resp.url, &cached) {
// 新
if let Err(e) = crate::storage::save_response(&*self.store, method_str, &resp.url, &cached) {
```

- [ ] **Step 11: 临时调整 Cargo.toml（rusqlite 暂时仍是必需依赖）**

此时 Cargo.toml 保持原状（rusqlite 仍是必需依赖），保证默认编译通过。feature 开关在 Task 3 单独添加。

- [ ] **Step 12: 运行编译验证**

```bash
cd /home/weng/wisp && cargo build
```
Expected: 编译通过，0 错误。

- [ ] **Step 13: 运行单元测试**

```bash
cd /home/weng/wisp && cargo test --lib storage
```
Expected: storage 模块所有测试通过（自由函数 + MemoryStore + FileStore + SqliteStore）。

- [ ] **Step 14: 运行调用方相关测试**

```bash
cd /home/weng/wisp && cargo test --lib crawl
cd /home/weng/wisp && cargo test --lib parser
```
Expected: 调用方测试通过，验证迁移正确。

- [ ] **Step 15: 运行 clippy**

```bash
cd /home/weng/wisp && cargo clippy --no-deps --lib 2>&1 | tail -5
```
Expected: 无新增错误（已有 missing_docs 警告可忽略）。

- [ ] **Step 16: Commit**

```bash
cd /home/weng/wisp && git add -A && git commit -m "refactor(storage): trait 缩小为底层 KV 原语 + 新增 FileStore + 自由函数迁移"
```

---

