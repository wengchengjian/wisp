//! 统一存储层：可插拔的持久化后端 trait + Memory/File/SQLite 实现。
//!
//! 三类用途（通过自由函数实现，trait 仅提供底层 KV 原语）：
//! - Checkpoint（断点续爬）：`save_checkpoint` / `load_checkpoint` / `delete_checkpoint`
//! - Element Snapshot（自适应定位）：`save_element` / `load_element`
//! - Response Cache（HTTP 响应缓存，带 per-entry TTL）：`save_response` / `load_response` / `delete_response`

#[cfg(feature = "sqlite")]
pub mod migrations;

mod file;
mod memory;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use file::FileStore;
pub use memory::MemoryStore;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

use crate::error::{Result, StorageError, WispError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

// ============================================================================
// Store trait：仅底层 KV 原语（async）
// ============================================================================

/// 存储后端 trait。仅提供底层 KV 原语，全部 `async`。
///
/// 实现者保证线程安全（`Send + Sync`）。SQLite/FileStore 等同步 I/O
/// 实现内部用 `tokio::task::spawn_blocking` 移出 async worker；
/// MemoryStore（moka 同步 API）直接 async 包装。
///
/// 业务方法（`save_checkpoint` / `load_response` 等）作为自由函数实现，
/// 调用 `set`/`get`/`delete` 并处理序列化。
#[async_trait]
pub trait Store: Send + Sync {
    /// 写入一个 entry。
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()>;

    /// 读取一个 entry。返回 `None` 表示不存在或已过期。
    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>>;

    /// 删除一个 entry。key 不存在不算错误。
    async fn delete(&self, namespace: &str, key: &str) -> Result<()>;

    /// 带 TTL 的写入。`ttl = None` 表示永不过期。
    ///
    /// 默认实现忽略 TTL（适用于不支持的存储）。
    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        _ttl: Option<Duration>,
    ) -> Result<()> {
        self.set(namespace, key, value).await
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
    #[must_use]
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
pub async fn save_checkpoint(store: &dyn Store, name: &str, state: &[u8]) -> Result<()> {
    store.set(NS_CHECKPOINT, name, state).await
}

/// 加载检查点。
pub async fn load_checkpoint(store: &dyn Store, name: &str) -> Result<Option<Vec<u8>>> {
    store.get(NS_CHECKPOINT, name).await
}

/// 删除检查点。
pub async fn delete_checkpoint(store: &dyn Store, name: &str) -> Result<()> {
    store.delete(NS_CHECKPOINT, name).await
}

// === Element Snapshot ===

/// 保存元素快照。
pub async fn save_element(
    store: &dyn Store,
    url: &str,
    key: &str,
    row: &ElementSnapshotRow,
) -> Result<()> {
    let composite = format!("{url}|{key}");
    let bytes = serde_json::to_vec(row).map_err(|e| {
        WispError::Storage(StorageError::Serialization(format!("serialize element: {e}")))
    })?;
    store.set(NS_ELEMENT, &composite, &bytes).await
}

/// 加载元素快照。
pub async fn load_element(
    store: &dyn Store,
    url: &str,
    key: &str,
) -> Result<Option<ElementSnapshotRow>> {
    let composite = format!("{url}|{key}");
    store
        .get(NS_ELEMENT, &composite)
        .await?
        .map(|v| serde_json::from_slice(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::Corrupted(format!("parse element: {e}"))))
}

// === Response Cache ===

/// 保存响应缓存。
pub async fn save_response(
    store: &dyn Store,
    method: &str,
    url: &str,
    resp: &CachedResponse,
) -> Result<()> {
    let composite = format!("{method}|{url}");
    let bytes = serde_json::to_vec(resp).map_err(|e| {
        WispError::Storage(StorageError::Serialization(format!("serialize response: {e}")))
    })?;
    store
        .set_with_ttl(NS_RESPONSE, &composite, &bytes, resp.ttl)
        .await
}

/// 加载响应缓存。
pub async fn load_response(
    store: &dyn Store,
    method: &str,
    url: &str,
) -> Result<Option<CachedResponse>> {
    let composite = format!("{method}|{url}");
    store
        .get(NS_RESPONSE, &composite)
        .await?
        .map(|v| serde_json::from_slice(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::Corrupted(format!("parse response: {e}"))))
}

/// 删除响应缓存。
pub async fn delete_response(store: &dyn Store, method: &str, url: &str) -> Result<()> {
    let composite = format!("{method}|{url}");
    store.delete(NS_RESPONSE, &composite).await
}

// ============================================================================
// 测试：自由函数 + MockStore
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// MockStore 内部数据类型：(namespace, key) → (value, optional expiry)
    type MockStoreData = HashMap<(String, String), (Vec<u8>, Option<Instant>)>;

    /// 测试用 MockStore：基于 HashMap，支持 TTL 检查。
    struct MockStore {
        data: Mutex<MockStoreData>,
    }

    impl MockStore {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    use std::time::Instant;

    #[async_trait]
    impl Store for MockStore {
        async fn set(&self, ns: &str, key: &str, value: &[u8]) -> Result<()> {
            self.data
                .lock()
                .insert((ns.into(), key.into()), (value.to_vec(), None));
            Ok(())
        }
        async fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>> {
            let now = Instant::now();
            let g = self.data.lock();
            if let Some((v, exp)) = g.get(&(ns.into(), key.into())) {
                if let Some(exp) = exp {
                    if now > *exp {
                        return Ok(None);
                    }
                }
                Ok(Some(v.clone()))
            } else {
                Ok(None)
            }
        }
        async fn delete(&self, ns: &str, key: &str) -> Result<()> {
            self.data.lock().remove(&(ns.into(), key.into()));
            Ok(())
        }
        async fn set_with_ttl(
            &self,
            ns: &str,
            key: &str,
            value: &[u8],
            ttl: Option<Duration>,
        ) -> Result<()> {
            let exp = ttl.map(|d| Instant::now() + d);
            self.data
                .lock()
                .insert((ns.into(), key.into()), (value.to_vec(), exp));
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

    #[tokio::test]
    async fn checkpoint_roundtrip_via_free_fn() {
        let store = MockStore::new();
        save_checkpoint(&store, "spider1", b"state-bytes")
            .await
            .unwrap();
        let loaded = load_checkpoint(&store, "spider1").await.unwrap().unwrap();
        assert_eq!(loaded, b"state-bytes");
        delete_checkpoint(&store, "spider1").await.unwrap();
        assert!(load_checkpoint(&store, "spider1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn response_roundtrip_via_free_fn() {
        let store = MockStore::new();
        let resp = make_cached(200, b"<html>hi</html>", Some(Duration::from_hours(1)));
        save_response(&store, "GET", "https://example.com", &resp)
            .await
            .unwrap();
        let loaded = load_response(&store, "GET", "https://example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.status, 200);
        assert_eq!(loaded.body, b"<html>hi</html>");
        assert_eq!(loaded.content_type, "text/html");
    }

    #[tokio::test]
    async fn response_ttl_expiry() {
        let store = MockStore::new();
        let resp = make_cached(200, b"x", Some(Duration::from_millis(1)));
        save_response(&store, "GET", "https://expired.com", &resp)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(load_response(&store, "GET", "https://expired.com")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn response_no_ttl_never_expires() {
        let store = MockStore::new();
        let resp = make_cached(200, b"forever", None);
        save_response(&store, "GET", "https://forever.com", &resp)
            .await
            .unwrap();
        let loaded = load_response(&store, "GET", "https://forever.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.body, b"forever");
    }

    #[tokio::test]
    async fn method_isolation() {
        let store = MockStore::new();
        save_response(
            &store,
            "GET",
            "https://example.com",
            &make_cached(200, b"get", None),
        )
        .await
        .unwrap();
        save_response(
            &store,
            "POST",
            "https://example.com",
            &make_cached(201, b"post", None),
        )
        .await
        .unwrap();
        assert_eq!(
            load_response(&store, "GET", "https://example.com")
                .await
                .unwrap()
                .unwrap()
                .body,
            b"get"
        );
        assert_eq!(
            load_response(&store, "POST", "https://example.com")
                .await
                .unwrap()
                .unwrap()
                .body,
            b"post"
        );
    }

    #[tokio::test]
    async fn namespace_isolation() {
        let store = MockStore::new();
        // checkpoint 和 element 同名 key 不冲突
        save_checkpoint(&store, "mykey", b"cp").await.unwrap();
        let elem = ElementSnapshotRow {
            tag: "div".into(),
            attrs: serde_json::Value::Null,
            text_preview: "hi".into(),
            ancestor_path: serde_json::Value::Null,
            sibling_tags: serde_json::Value::Null,
            position_in_parent: 0,
            parent_tag: "body".into(),
            parent_attrs: serde_json::Value::Null,
            captured_at: 0,
        };
        save_element(&store, "http://x", "mykey", &elem)
            .await
            .unwrap();
        assert_eq!(
            load_checkpoint(&store, "mykey").await.unwrap().unwrap(),
            b"cp"
        );
        assert!(load_element(&store, "http://x", "mykey")
            .await
            .unwrap()
            .is_some());
    }
}
