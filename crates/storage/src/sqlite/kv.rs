//! Store trait 的 SQLite KV 实现。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::params;

use super::SqliteStore;
use crate::Store;
use wisp_core::error::{Result, StorageError, WispError};

#[async_trait]
impl Store for SqliteStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
                 VALUES (?1, ?2, ?3, NULL, ?4)",
                params![namespace, key, value, now],
            )
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok(())
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT value FROM kv
                     WHERE namespace = ?1 AND key = ?2
                       AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
                )
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let mut rows = stmt
                .query(params![namespace, key])
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            if let Some(row) = rows
                .next()
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?
            {
                let value: Vec<u8> = row
                    .get(0)
                    .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
                Ok(Some(value))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.execute(
                "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
                params![namespace, key],
            )
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok(())
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.to_vec();
        let ttl_secs = ttl.map(|d| d.as_secs() as i64);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![namespace, key, value, ttl_secs, now],
            )
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok(())
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }
}
