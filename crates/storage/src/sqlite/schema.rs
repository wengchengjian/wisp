//! SQLite schema 初始化与旧 schema 检测。

use super::SqliteStore;
use crate::migrations::SCHEMA_V1;
use wisp_core::error::{Result, StorageError, WispError};

impl SqliteStore {
    pub(super) fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;

        // 旧 schema 检测：如果存在旧三表，打印 warning 提示数据已弃用
        let has_old_table: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name IN ('element_snapshots', 'crawl_checkpoints', 'response_cache')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if has_old_table {
            tracing::warn!("检测到旧 schema (element_snapshots/crawl_checkpoints/response_cache 三表)，与新版单表 kv 结构不兼容。旧数据已弃用，建议删除 db 文件重新开始。");
        }

        conn.execute_batch(SCHEMA_V1)
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
        Ok(())
    }
}
