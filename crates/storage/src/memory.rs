//! 内存存储后端。单 moka 实例，per-entry TTL 原生支持。

use async_trait::async_trait;
use moka::sync::Cache as MokaCache;
use moka::Expiry;
use std::time::{Duration, Instant};

use super::Store;
use wisp_core::error::Result;

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
        entry
            .expires_at
            .map(|at| at.saturating_duration_since(_now))
    }
}

/// 内存存储后端。
///
/// 单 moka 实例，capacity 限制总 entry 数（默认 100_000）。
/// TTL 通过 `set_with_ttl` 写入 entry 的 `expires_at` 字段，moka 在过期时自动淘汰。
///
/// moka::sync::Cache 的方法都是同步非阻塞的，故直接用 `async fn` 包装，
/// 不需要 `spawn_blocking`。
pub struct MemoryStore {
    inner: MokaCache<(String, String), Entry>,
}

impl MemoryStore {
    /// 创建内存存储。`capacity` 限制总 entry 数。
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

#[async_trait]
impl Store for MemoryStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let entry = Entry {
            value: value.to_vec(),
            expires_at: None,
        };
        self.inner
            .insert((namespace.to_string(), key.to_string()), entry);
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .inner
            .get(&(namespace.to_string(), key.to_string()))
            .map(|e| e.value))
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        self.inner
            .invalidate(&(namespace.to_string(), key.to_string()));
        Ok(())
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let expires_at = ttl.map(|d| Instant::now() + d);
        let entry = Entry {
            value: value.to_vec(),
            expires_at,
        };
        self.inner
            .insert((namespace.to_string(), key.to_string()), entry);
        Ok(())
    }
}
