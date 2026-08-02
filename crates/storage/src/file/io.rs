//! Store trait 的同步文件 I/O 实现。

use std::fs;
use std::io::Read;

use async_trait::async_trait;

use super::FileStore;
use super::path::path_for;
use super::ttl::{pack_with_ttl, unpack_and_check};
use crate::Store;
use wisp_core::error::{Result, StorageError, WispError};

#[async_trait]
impl Store for FileStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let path = path_for(&self.root, namespace, key);
        let value = value.to_vec();
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    WispError::Storage(StorageError::General(format!("mkdir: {e}")))
                })?;
            }
            let packed = pack_with_ttl(&value, None);
            fs::write(&path, packed)
                .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
            Ok(())
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let path = path_for(&self.root, namespace, key);
        tokio::task::spawn_blocking(move || {
            let mut file = match fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => {
                    return Err(WispError::Storage(StorageError::General(format!(
                        "open: {e}"
                    ))));
                }
            };
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| WispError::Storage(StorageError::General(format!("read: {e}"))))?;
            match unpack_and_check(&buf) {
                Some(v) => Ok(Some(v)),
                None => {
                    // 过期或损坏：惰性删除
                    let _ = fs::remove_file(&path);
                    Ok(None)
                }
            }
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let path = path_for(&self.root, namespace, key);
        tokio::task::spawn_blocking(move || match fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(WispError::Storage(StorageError::General(format!(
                "delete: {e}"
            )))),
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
        ttl: Option<std::time::Duration>,
    ) -> Result<()> {
        let path = path_for(&self.root, namespace, key);
        let value = value.to_vec();
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    WispError::Storage(StorageError::General(format!("mkdir: {e}")))
                })?;
            }
            let packed = pack_with_ttl(&value, ttl);
            fs::write(&path, packed)
                .map_err(|e| WispError::Storage(StorageError::General(format!("write: {e}"))))?;
            Ok(())
        })
        .await
        .map_err(|e| {
            WispError::Storage(StorageError::General(format!("spawn_blocking join: {e}")))
        })?
    }
}
