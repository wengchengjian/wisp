//! Store trait：仅底层 KV 原语（async）。

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use wisp_core::error::{Result, StorageError, WispError};

use crate::FileStore;
#[cfg(feature = "sqlite")]
use crate::SqliteStore;
use crate::models::CachedResponse;

const NS_CHECKPOINT: &str = "checkpoint";
const NS_RESPONSE: &str = "response";

fn response_key(method: &str, url: &str) -> String {
    format!("{method}|{url}")
}

/// 存储后端 trait。底层 KV 原语 + 业务默认方法，全部 `async`。
///
/// 实现者保证线程安全（`Send + Sync`）。SQLite/FileStore 等同步 I/O
/// 实现内部用 `tokio::task::spawn_blocking` 移出 async worker；
/// MemoryStore（moka 同步 API）直接 async 包装。
///
/// 业务方法（`save_checkpoint` / `load_response` 等）以默认方法实现，
/// 调用 `set`/`get`/`delete` 并处理序列化，无需后端重新实现。
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

    // === Checkpoint ===

    /// 保存检查点。
    async fn save_checkpoint(&self, name: &str, state: &[u8]) -> Result<()> {
        self.set(NS_CHECKPOINT, name, state).await
    }

    /// 加载检查点。
    async fn load_checkpoint(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.get(NS_CHECKPOINT, name).await
    }

    /// 删除检查点。
    async fn delete_checkpoint(&self, name: &str) -> Result<()> {
        self.delete(NS_CHECKPOINT, name).await
    }

    // === Response Cache ===

    /// 保存响应缓存。
    ///
    /// 使用 bincode 紧凑二进制而非 JSON：响应体是 `Vec<u8>`，JSON 会展开成
    /// 字节数组，序列化/反序列化开销和缓存体积都远高于二进制编码。
    async fn save_response(&self, method: &str, url: &str, resp: &CachedResponse) -> Result<()> {
        let composite = response_key(method, url);
        let bytes = bincode::serialize(resp).map_err(|e| {
            WispError::Storage(StorageError::General(format!("serialize response: {e}")))
        })?;
        self.set_with_ttl(NS_RESPONSE, &composite, &bytes, resp.ttl)
            .await
    }

    /// 加载响应缓存。
    async fn load_response(&self, method: &str, url: &str) -> Result<Option<CachedResponse>> {
        let composite = response_key(method, url);
        self.get(NS_RESPONSE, &composite)
            .await?
            .map(|v| bincode::deserialize(&v))
            .transpose()
            .map_err(|e| WispError::Storage(StorageError::General(format!("parse response: {e}"))))
    }

    /// 删除响应缓存。
    async fn delete_response(&self, method: &str, url: &str) -> Result<()> {
        let composite = response_key(method, url);
        self.delete(NS_RESPONSE, &composite).await
    }
}

/// 按路径选择并打开存储后端。
///
/// 空路径或 `:memory:` 使用默认 `FileStore`；其他路径在 sqlite 特性下打开
/// SQLite 数据库文件，否则回退为 `FileStore` 目录。
pub fn open_store(path: &str) -> Result<Arc<dyn Store>> {
    if path.is_empty() || path == ":memory:" {
        return Ok(Arc::new(FileStore::default()));
    }
    #[cfg(feature = "sqlite")]
    {
        Ok(Arc::new(SqliteStore::open(std::path::Path::new(path))?))
    }
    #[cfg(not(feature = "sqlite"))]
    {
        tracing::warn!("当前构建未启用 sqlite，使用 FileStore 目录: {path}");
        Ok(Arc::new(FileStore::with_dir(std::path::PathBuf::from(
            path,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_store_memory_roundtrips() {
        let store = open_store(":memory:").expect("open memory store");
        store.set("ns", "k_memory", b"v").await.expect("set");
        assert_eq!(
            store.get("ns", "k_memory").await.expect("get").as_deref(),
            Some(&b"v"[..])
        );
    }

    #[tokio::test]
    async fn open_store_empty_path_roundtrips() {
        let store = open_store("").expect("open default store");
        store.set("ns", "k_empty", b"v").await.expect("set");
        assert_eq!(
            store.get("ns", "k_empty").await.expect("get").as_deref(),
            Some(&b"v"[..])
        );
    }

    #[tokio::test]
    async fn open_store_path_roundtrips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store");
        let store = open_store(path.to_str().expect("utf8")).expect("open path store");
        store.set("ns", "k", b"v").await.expect("set");
        assert_eq!(
            store.get("ns", "k").await.expect("get").as_deref(),
            Some(&b"v"[..])
        );
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn open_store_sqlite_creates_db_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.db");
        let store = open_store(path.to_str().expect("utf8")).expect("open sqlite store");
        store.set("ns", "k", b"v").await.expect("set");
        assert!(path.exists(), "sqlite 应创建数据库文件");
    }

    #[cfg(not(feature = "sqlite"))]
    #[tokio::test]
    async fn open_store_file_creates_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store");
        let store = open_store(path.to_str().expect("utf8")).expect("open file store");
        store.set("ns", "k", b"v").await.expect("set");
        assert!(path.is_dir(), "file store 应创建目录");
    }
}
