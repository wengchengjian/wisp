//! Unified SQLite storage layer.
//!
//! Single database file shared by adaptive (element_snapshots) and
//! crawl checkpoint (crawl_checkpoints) modules. Stage 4 will add
//! session_cookies and response_cache tables.

pub mod migrations;

use std::path::Path;
use rusqlite::{params, Connection};
use crate::error::{Result, WispError};

/// Unified SQLite store. Inner connection is NOT thread-safe by itself;
/// callers wrap it in `Arc<Mutex<Store>>` for concurrent access.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open or create the database file at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| WispError::Storage(e.to_string()))?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| WispError::Storage(e.to_string()))?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(migrations::SCHEMA_V1)
            .map_err(|e| WispError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Raw connection accessor (for module-internal queries).
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}
