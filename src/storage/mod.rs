//! 统一存储层：可插拔的持久化后端 trait + Memory/SQLite 实现。
//!
//! 三类用途：
//! - Checkpoint（断点续爬）：`save/load/delete_checkpoint`
//! - Element Snapshot（自适应定位）：`save/load_element`
//! - Response Cache（HTTP 响应缓存，带 per-entry TTL）：`save/load/delete_response`

pub mod migrations;

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use moka::sync::Cache as MokaCache;
use moka::Expiry;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::{Result, WispError, StorageError};

// ============================================================================
// Trait 定义
// ============================================================================

/// 存储后端 trait。所有方法同步——SQLite/HashMap 操作足够快，无需 async。
///
/// 实现者保证线程安全（`Send + Sync`），内部用 `parking_lot::Mutex` 或 moka 保护。
pub trait Store: Send + Sync {
    // === Checkpoint ===
    fn save_checkpoint(&self, spider_name: &str, state_bytes: &[u8], saved_at: i64) -> Result<()>;
    fn load_checkpoint(&self, spider_name: &str) -> Result<Option<Vec<u8>>>;
    fn delete_checkpoint(&self, spider_name: &str) -> Result<()>;

    // === Element Snapshot ===
    fn save_element(&self, url: &str, key: &str, row: &ElementSnapshotRow) -> Result<()>;
    fn load_element(&self, url: &str, key: &str) -> Result<Option<ElementSnapshotRow>>;

    // === Response Cache ===
    fn save_response(&self, method: &str, url: &str, resp: &CachedResponse) -> Result<()>;
    fn load_response(&self, method: &str, url: &str) -> Result<Option<CachedResponse>>;
    fn delete_response(&self, method: &str, url: &str) -> Result<()>;
}

// ============================================================================
// 公共数据类型
// ============================================================================

/// 可缓存的响应数据（`Response` 的可序列化子集）。
///
/// 不含 `request` 字段——命中时由 `CacheMiddleware` 用当前请求重建完整 `Response`。
/// `cached_at` + `ttl` 配对决定过期时间。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
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
// SqliteStore
// ============================================================================

/// SQLite 存储后端。线程安全（`parking_lot::Mutex<Connection>`，无 poison）。
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// 打开或创建数据库文件。
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let store = Self { conn: Mutex::new(conn) };
        store.init_schema()?;
        Ok(store)
    }

    /// 内存数据库（测试用）。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
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
        conn.execute_batch(migrations::SCHEMA_V1)
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }
}

impl Store for SqliteStore {
    fn save_checkpoint(&self, spider_name: &str, state_bytes: &[u8], saved_at: i64) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO crawl_checkpoints (spider_name, state, saved_at) VALUES (?1, ?2, ?3)",
            params![spider_name, state_bytes, saved_at],
        ).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }

    fn load_checkpoint(&self, spider_name: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT state FROM crawl_checkpoints WHERE spider_name = ?1")
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let mut rows = stmt.query(params![spider_name]).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        if let Some(row) = rows.next().map_err(|e| WispError::Storage(StorageError::General(e.to_string())))? {
            let blob: Vec<u8> = row.get(0).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok(Some(blob))
        } else {
            Ok(None)
        }
    }

    fn delete_checkpoint(&self, spider_name: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM crawl_checkpoints WHERE spider_name = ?1", params![spider_name])
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }

    fn save_element(&self, url: &str, key: &str, row: &ElementSnapshotRow) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO element_snapshots
             (url, key, tag, attrs, text_preview, ancestor_path, sibling_tags,
              position_in_parent, parent_tag, parent_attrs, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                url, key, row.tag,
                row.attrs.to_string(), row.text_preview,
                row.ancestor_path.to_string(), row.sibling_tags.to_string(),
                row.position_in_parent, row.parent_tag, row.parent_attrs.to_string(),
                row.captured_at,
            ],
        ).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }

    fn load_element(&self, url: &str, key: &str) -> Result<Option<ElementSnapshotRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT tag, attrs, text_preview, ancestor_path, sibling_tags,
                    position_in_parent, parent_tag, parent_attrs, captured_at
             FROM element_snapshots WHERE url = ?1 AND key = ?2"
        ).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let mut rows = stmt.query(params![url, key]).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        if let Some(row) = rows.next().map_err(|e| WispError::Storage(StorageError::General(e.to_string())))? {
            Ok(Some(ElementSnapshotRow {
                tag: row.get(0).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?,
                attrs: serde_json::from_str(&row.get::<_, String>(1).unwrap_or_default()).unwrap_or_default(),
                text_preview: row.get(2).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?,
                ancestor_path: serde_json::from_str(&row.get::<_, String>(3).unwrap_or_default()).unwrap_or_default(),
                sibling_tags: serde_json::from_str(&row.get::<_, String>(4).unwrap_or_default()).unwrap_or_default(),
                position_in_parent: row.get(5).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?,
                parent_tag: row.get(6).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?,
                parent_attrs: serde_json::from_str(&row.get::<_, String>(7).unwrap_or_default()).unwrap_or_default(),
                captured_at: row.get(8).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?,
            }))
        } else {
            Ok(None)
        }
    }

    fn save_response(&self, method: &str, url: &str, resp: &CachedResponse) -> Result<()> {
        let conn = self.conn.lock();
        let ttl_secs = resp.ttl.map(|d| d.as_secs() as i64);
        conn.execute(
            "INSERT OR REPLACE INTO response_cache
             (url, method, status, headers, body, content_type, cached_at, ttl_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                url, method, resp.status,
                serde_json::to_string(&resp.headers).unwrap_or_default(),
                resp.body, resp.content_type, resp.cached_at, ttl_secs,
            ],
        ).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }

    fn load_response(&self, method: &str, url: &str) -> Result<Option<CachedResponse>> {
        let conn = self.conn.lock();
        // SQL 层过滤过期 entry：ttl_secs IS NULL 表示永不过期。
        // 注意：strftime('%s','now') 返回 TEXT，而 cached_at + ttl_secs 是 INTEGER，
        // SQLite 类型比较规则下 INTEGER < TEXT 恒成立，必须 CAST 为 INTEGER 才能正确比较。
        let mut stmt = conn.prepare(
            "SELECT status, headers, body, content_type, cached_at, ttl_secs
             FROM response_cache
             WHERE url = ?1 AND method = ?2
               AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))"
        ).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let mut rows = stmt.query(params![url, method]).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        if let Some(row) = rows.next().map_err(|e| WispError::Storage(StorageError::General(e.to_string())))? {
            let status: i64 = row.get(0).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let headers_str: String = row.get(1).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let body: Vec<u8> = row.get(2).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let content_type: String = row.get(3).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let cached_at: i64 = row.get(4).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let ttl_secs: Option<i64> = row.get(5).map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok(Some(CachedResponse {
                status: status as u16,
                headers: serde_json::from_str(&headers_str).unwrap_or_default(),
                body,
                content_type,
                cached_at,
                ttl: ttl_secs.map(|s| Duration::from_secs(s as u64)),
            }))
        } else {
            Ok(None)
        }
    }

    fn delete_response(&self, method: &str, url: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM response_cache WHERE url = ?1 AND method = ?2", params![url, method])
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }
}

// ============================================================================
// MemoryStore
// ============================================================================

/// per-entry TTL 过期策略：从 `CachedResponse.ttl` 读取有效期。
struct ResponseExpiry;

impl Expiry<(String, String), CachedResponse> for ResponseExpiry {
    fn expire_after_create(
        &self,
        _key: &(String, String),
        value: &CachedResponse,
        _created_at: Instant,
    ) -> Option<Duration> {
        value.ttl
    }

    fn expire_after_update(
        &self,
        key: &(String, String),
        value: &CachedResponse,
        updated_at: Instant,
        _prev: Option<Duration>,
    ) -> Option<Duration> {
        self.expire_after_create(key, value, updated_at)
    }
}

/// 内存存储后端（测试用，零 IO）。
///
/// checkpoint/element 用 `parking_lot::Mutex<HashMap>`，
/// response 缓存用 `moka::sync::Cache`（支持 per-entry TTL + 容量淘汰）。
pub struct MemoryStore {
    checkpoints: Mutex<HashMap<String, Vec<u8>>>,
    elements: Mutex<HashMap<(String, String), ElementSnapshotRow>>,
    responses: MokaCache<(String, String), CachedResponse>,
}

impl MemoryStore {
    /// 创建内存存储。`max_response_entries` 限制响应缓存条目数（默认 10000）。
    pub fn new(max_response_entries: u64) -> Self {
        Self {
            checkpoints: Mutex::new(HashMap::new()),
            elements: Mutex::new(HashMap::new()),
            responses: MokaCache::builder()
                .max_capacity(max_response_entries)
                .expire_after(ResponseExpiry)
                .build(),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new(10000)
    }
}

impl Store for MemoryStore {
    fn save_checkpoint(&self, spider_name: &str, state_bytes: &[u8], _saved_at: i64) -> Result<()> {
        self.checkpoints.lock().insert(spider_name.to_string(), state_bytes.to_vec());
        Ok(())
    }

    fn load_checkpoint(&self, spider_name: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.checkpoints.lock().get(spider_name).cloned())
    }

    fn delete_checkpoint(&self, spider_name: &str) -> Result<()> {
        self.checkpoints.lock().remove(spider_name);
        Ok(())
    }

    fn save_element(&self, url: &str, key: &str, row: &ElementSnapshotRow) -> Result<()> {
        self.elements.lock().insert((url.to_string(), key.to_string()), row.clone());
        Ok(())
    }

    fn load_element(&self, url: &str, key: &str) -> Result<Option<ElementSnapshotRow>> {
        Ok(self.elements.lock().get(&(url.to_string(), key.to_string())).cloned())
    }

    fn save_response(&self, method: &str, url: &str, resp: &CachedResponse) -> Result<()> {
        self.responses.insert((method.to_string(), url.to_string()), resp.clone());
        Ok(())
    }

    fn load_response(&self, method: &str, url: &str) -> Result<Option<CachedResponse>> {
        Ok(self.responses.get(&(method.to_string(), url.to_string())))
    }

    fn delete_response(&self, method: &str, url: &str) -> Result<()> {
        self.responses.invalidate(&(method.to_string(), url.to_string()));
        Ok(())
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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

    // --- SqliteStore ---

    #[test]
    fn sqlite_checkpoint_roundtrip() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.save_checkpoint("spider1", b"state-bytes", 1234567890).unwrap();
        let loaded = store.load_checkpoint("spider1").unwrap().unwrap();
        assert_eq!(loaded, b"state-bytes");
        store.delete_checkpoint("spider1").unwrap();
        assert!(store.load_checkpoint("spider1").unwrap().is_none());
    }

    #[test]
    fn sqlite_response_roundtrip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let resp = make_cached(200, b"<html>hi</html>", Some(Duration::from_secs(3600)));
        store.save_response("GET", "https://example.com", &resp).unwrap();
        let loaded = store.load_response("GET", "https://example.com").unwrap().unwrap();
        assert_eq!(loaded.status, 200);
        assert_eq!(loaded.body, b"<html>hi</html>");
        assert_eq!(loaded.content_type, "text/html");
    }

    #[test]
    fn sqlite_response_expired_filtered() {
        let store = SqliteStore::open_in_memory().unwrap();
        // 手动写入已过期 entry（cached_at = 1，ttl = 1 秒，早就过期了）
        let expired = CachedResponse {
            status: 200, headers: HashMap::new(), body: b"old".to_vec(),
            content_type: String::new(), cached_at: 1, ttl: Some(Duration::from_secs(1)),
        };
        store.save_response("GET", "https://expired.com", &expired).unwrap();
        assert!(store.load_response("GET", "https://expired.com").unwrap().is_none());
    }

    #[test]
    fn sqlite_response_no_ttl_never_expires() {
        let store = SqliteStore::open_in_memory().unwrap();
        let resp = make_cached(200, b"forever", None);
        store.save_response("GET", "https://forever.com", &resp).unwrap();
        let loaded = store.load_response("GET", "https://forever.com").unwrap().unwrap();
        assert_eq!(loaded.body, b"forever");
    }

    // --- MemoryStore ---

    #[test]
    fn memory_checkpoint_roundtrip() {
        let store = MemoryStore::default();
        store.save_checkpoint("spider1", b"state", 0).unwrap();
        assert_eq!(store.load_checkpoint("spider1").unwrap().unwrap(), b"state");
        store.delete_checkpoint("spider1").unwrap();
        assert!(store.load_checkpoint("spider1").unwrap().is_none());
    }

    #[test]
    fn memory_response_roundtrip() {
        let store = MemoryStore::default();
        let resp = make_cached(200, b"hello", Some(Duration::from_secs(60)));
        store.save_response("GET", "https://example.com", &resp).unwrap();
        let loaded = store.load_response("GET", "https://example.com").unwrap().unwrap();
        assert_eq!(loaded.body, b"hello");
    }

    #[test]
    fn memory_response_delete() {
        let store = MemoryStore::default();
        let resp = make_cached(200, b"x", None);
        store.save_response("POST", "https://example.com/api", &resp).unwrap();
        assert!(store.load_response("POST", "https://example.com/api").unwrap().is_some());
        store.delete_response("POST", "https://example.com/api").unwrap();
        assert!(store.load_response("POST", "https://example.com/api").unwrap().is_none());
    }

    #[test]
    fn memory_method_isolation() {
        let store = MemoryStore::default();
        store.save_response("GET", "https://example.com", &make_cached(200, b"get", None)).unwrap();
        store.save_response("POST", "https://example.com", &make_cached(201, b"post", None)).unwrap();
        assert_eq!(store.load_response("GET", "https://example.com").unwrap().unwrap().body, b"get");
        assert_eq!(store.load_response("POST", "https://example.com").unwrap().unwrap().body, b"post");
    }

    #[test]
    fn cached_response_is_expired_logic() {
        let now = chrono::Utc::now().timestamp();
        let fresh = CachedResponse { status: 200, headers: HashMap::new(), body: vec![], content_type: String::new(), cached_at: now, ttl: Some(Duration::from_secs(3600)) };
        assert!(!fresh.is_expired());

        let expired = CachedResponse { status: 200, headers: HashMap::new(), body: vec![], content_type: String::new(), cached_at: 1, ttl: Some(Duration::from_secs(1)) };
        assert!(expired.is_expired());

        let forever = CachedResponse { status: 200, headers: HashMap::new(), body: vec![], content_type: String::new(), cached_at: 1, ttl: None };
        assert!(!forever.is_expired());
    }
}
