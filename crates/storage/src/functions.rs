//! 存储层业务自由函数。

use crate::models::{CachedResponse, ElementSnapshotRow};
use crate::store::Store;
use wisp_core::error::{Result, StorageError, WispError};

const NS_CHECKPOINT: &str = "checkpoint";
const NS_ELEMENT: &str = "element";
const NS_RESPONSE: &str = "response";

fn element_key(url: &str, key: &str) -> String {
    format!("{url}|{key}")
}

fn response_key(method: &str, url: &str) -> String {
    format!("{method}|{url}")
}

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
    let composite = element_key(url, key);
    let bytes = serde_json::to_vec(row).map_err(|e| {
        WispError::Storage(StorageError::General(format!("serialize element: {e}")))
    })?;
    store.set(NS_ELEMENT, &composite, &bytes).await
}

/// 加载元素快照。
pub async fn load_element(
    store: &dyn Store,
    url: &str,
    key: &str,
) -> Result<Option<ElementSnapshotRow>> {
    let composite = element_key(url, key);
    store
        .get(NS_ELEMENT, &composite)
        .await?
        .map(|v| serde_json::from_slice(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::General(format!("parse element: {e}"))))
}

// === Response Cache ===

/// 保存响应缓存。
///
/// 使用 bincode 紧凑二进制而非 JSON：响应体是 `Vec<u8>`，JSON 会展开成
/// 字节数组，序列化/反序列化开销和缓存体积都远高于二进制编码。
pub async fn save_response(
    store: &dyn Store,
    method: &str,
    url: &str,
    resp: &CachedResponse,
) -> Result<()> {
    let composite = response_key(method, url);
    let bytes = bincode::serialize(resp).map_err(|e| {
        WispError::Storage(StorageError::General(format!("serialize response: {e}")))
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
    let composite = response_key(method, url);
    store
        .get(NS_RESPONSE, &composite)
        .await?
        .map(|v| bincode::deserialize(&v))
        .transpose()
        .map_err(|e| WispError::Storage(StorageError::General(format!("parse response: {e}"))))
}

/// 删除响应缓存。
pub async fn delete_response(store: &dyn Store, method: &str, url: &str) -> Result<()> {
    let composite = response_key(method, url);
    store.delete(NS_RESPONSE, &composite).await
}

// ============================================================================
// 测试：自由函数 + MockStore
// ============================================================================
