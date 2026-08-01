//! SQLite 存储后端。单表 KV 结构。
//!
//! 所有同步 I/O 用 `tokio::task::spawn_blocking` 包装移出 async worker，
//! 避免阻塞 runtime。`conn` 用 `Arc<Mutex<Connection>>` 让闭包 `'static`。

mod kv;
mod schema;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

use wisp_core::error::{Result, StorageError, WispError};

/// SQLite 存储后端。线程安全（`Arc<parking_lot::Mutex<Connection>>`，无 poison）。
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// 打开或创建数据库文件。
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    /// 内存数据库（测试用）。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }
}
