//! 统一存储层：可插拔的持久化后端 trait + Memory/File/SQLite 实现。
//!
//! 三类用途（通过自由函数实现，trait 仅提供底层 KV 原语）：
//! - Checkpoint（断点续爬）：`save_checkpoint` / `load_checkpoint` / `delete_checkpoint`
//! - Element Snapshot（自适应定位）：`save_element` / `load_element`
//! - Response Cache（HTTP 响应缓存，带 per-entry TTL）：`save_response` / `load_response` / `delete_response`

#[cfg(feature = "sqlite")]
pub mod migrations;

mod memory;
mod file;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use memory::MemoryStore;
pub use file::FileStore;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

use std::time::Duration;
use serde::{Deserialize, Serialize};
use wisp_core::error::{Result, StorageError, WispError};

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
    /// HTTP 状态码。
    pub status: u16,
    /// 响应头。
    pub headers: std::collections::HashMap<String, String>,
    /// 响应体。
    pub body: Vec<u8>,
    /// 内容类型。
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
    /// 元素标签名。
    pub tag: String,
    /// 元素属性。
    pub attrs: serde_json::Value,
    /// 文本预览。
    pub text_preview: String,
    /// 祖先路径。
    pub ancestor_path: serde_json::Value,
    /// 兄弟标签。
    pub sibling_tags: serde_json::Value,
    /// 在父节点中的位置。
    pub position_in_parent: i64,
    /// 父节点标签。
    pub parent_tag: String,
    /// 父节点属性。
    pub parent_attrs: serde_json::Value,
    /// 捕获时间（Unix 秒）。
    pub captured_at: i64,
}

// ============================================================================
// 业务层自由函数
// ============================================================================

const NS_CHECKPOINT: &str = "checkpoint";
const NS_ELEMENT: &str = "element";
const NS_RESPONSE: &str = "response";

// === Checkpoint ===

/// 保存检查点。
pub fn save_checkpoint(store: &dyn Store, name: &str, state: &[u8]) -> Result<()> {
    store.set(NS_CHECKPOINT, name, state)
}

/// 加载检查点。
pub fn load_checkpoint(store: &dyn Store, name: &str) -> Result<Option<Vec<u8>>> {
    store.get(NS_CHECKPOINT, name)
}

/// 删除检查点。
pub fn delete_checkpoint(store: &dyn Store, name: &str) -> Result<()> {
    store.delete(NS_CHECKPOINT, name)
}

// === Element Snapshot ===

/// 保存元素快照。
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

/// 加载元素快照。
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

/// 保存响应缓存。
///
/// 使用 bincode 紧凑二进制而非 JSON：响应体是 `Vec<u8>`，JSON 会展开成
/// 字节数组，序列化/反序列化开销和缓存体积都远高于二进制编码。
pub fn save_response(
    store: &dyn Store,
    method: &str,
    url: &str,
    resp: &CachedResponse,
) -> Result<()> {
    let composite = format!("{method}|{url}");
    let bytes = bincode::serialize(resp).map_err(|e| {
        WispError::Storage(StorageError::General(format!("serialize response: {e}")))
    })?;
    store.set_with_ttl(NS_RESPONSE, &composite, &bytes, resp.ttl)
}

/// 加载响应缓存。
pub fn load_response(
    store: &dyn Store,
    method: &str,
    url: &str,
) -> Result<Option<CachedResponse>> {
    let composite = format!("{method}|{url}");
    store
        .get(NS_RESPONSE, &composite)?
        .map(|v| bincode::deserialize(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::General(format!("parse response: {e}"))))
}

/// 删除响应缓存。
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
