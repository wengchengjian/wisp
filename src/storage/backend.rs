//! 存储抽象层 — 可插拔的键值存储后端。
//!
//! 借鉴 Crawlee Storage 设计：统一接口支持 Memory/SQLite/Redis/S3 等后端。
//! 现有 `Store` 的方法（checkpoint/cache）可迁移到调用此 trait。
//!
//! # 迁移路径
//!
//! - 不传 backend 时默认使用 SqliteBackend（包装现有 Store）
//! - 测试场景使用 MemoryBackend（零 IO）
//! - 分布式场景可接 RedisBackend（未来扩展）

use std::collections::HashMap;
use std::sync::Mutex;
use async_trait::async_trait;

use crate::error::Result;

/// 存储后端 trait：统一的键值存储接口。
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// 获取值。
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    /// 设置值。
    async fn set(&self, key: &str, value: &[u8]) -> Result<()>;
    /// 删除键。
    async fn delete(&self, key: &str) -> Result<()>;
    /// 列出指定前缀的所有键。
    async fn keys(&self, prefix: &str) -> Result<Vec<String>>;
}

/// 内存存储后端（测试用，零 IO）。
pub struct MemoryBackend {
    data: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self { data: Mutex::new(HashMap::new()) }
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StorageBackend for MemoryBackend {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.data.lock().unwrap().get(key).cloned())
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.data.lock().unwrap().insert(key.to_string(), value.to_vec());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.data.lock().unwrap().remove(key);
        Ok(())
    }

    async fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self.data.lock().unwrap().keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

/// SQLite 存储后端（包装现有 Store）。
pub struct SqliteBackend {
    store: super::Store,
}

impl SqliteBackend {
    /// 从已有 Store 创建。
    pub fn new(store: super::Store) -> Self {
        Self { store }
    }

    /// 打开数据库文件。
    pub fn open(path: &std::path::Path) -> Result<Self> {
        Ok(Self { store: super::Store::open(path)? })
    }

    /// 获取内部 Store 引用。
    pub fn store(&self) -> &super::Store {
        &self.store
    }
}

#[async_trait]
impl StorageBackend for SqliteBackend {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        // 使用 response_cache 表作为通用 KV 存储
        self.store.load_cached_response(key, "KV")
            .map(|opt| opt.map(|r| r.body))
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let cached = super::CachedResponse {
            status: 0,
            headers: HashMap::new(),
            body: value.to_vec(),
            cached_at: chrono::Utc::now().timestamp(),
        };
        self.store.save_cached_response(key, "KV", &cached)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        // SQLite 后端通过覆盖为空实现逻辑删除
        let cached = super::CachedResponse {
            status: 0,
            headers: HashMap::new(),
            body: vec![],
            cached_at: 0,
        };
        self.store.save_cached_response(key, "KV", &cached)
    }

    async fn keys(&self, _prefix: &str) -> Result<Vec<String>> {
        // SQLite 后端暂不支持前缀查询（需要专用 KV 表）
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_backend_basic() {
        let backend = MemoryBackend::new();
        assert_eq!(backend.get("key1").await.unwrap(), None);

        backend.set("key1", b"value1").await.unwrap();
        assert_eq!(backend.get("key1").await.unwrap(), Some(b"value1".to_vec()));

        backend.delete("key1").await.unwrap();
        assert_eq!(backend.get("key1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_memory_backend_keys() {
        let backend = MemoryBackend::new();
        backend.set("prefix:a", b"1").await.unwrap();
        backend.set("prefix:b", b"2").await.unwrap();
        backend.set("other:c", b"3").await.unwrap();

        let keys = backend.keys("prefix:").await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"prefix:a".to_string()));
        assert!(keys.contains(&"prefix:b".to_string()));
    }

    #[tokio::test]
    async fn test_sqlite_backend_basic() {
        let store = super::super::Store::open_in_memory().unwrap();
        let backend = SqliteBackend::new(store);

        backend.set("test_key", b"test_value").await.unwrap();
        let val = backend.get("test_key").await.unwrap();
        assert_eq!(val, Some(b"test_value".to_vec()));
    }
}
