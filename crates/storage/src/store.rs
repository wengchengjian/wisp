//! Store trait：仅底层 KV 原语（async）。

use async_trait::async_trait;
use std::time::Duration;
use wisp_core::error::Result;

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
