//! Unified SQLite storage layer.
//!
//! Single database file shared by adaptive (element_snapshots) and
//! crawl checkpoint (crawl_checkpoints) modules. Stage 4 will add
//! session_cookies and response_cache tables.

pub mod migrations;

use std::path::Path;
use rusqlite::{params, Connection};
use serde_json;
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

/// Element snapshot row (storage layer doesn't know about parser::Node).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ElementSnapshotRow {
    pub tag: String,
    pub attrs: serde_json::Value,        // JSON map
    pub text_preview: String,
    pub ancestor_path: serde_json::Value, // JSON array of strings
    pub sibling_tags: serde_json::Value, // JSON array of strings
    pub position_in_parent: i64,
    pub parent_tag: String,
    pub parent_attrs: serde_json::Value, // JSON map
    pub captured_at: i64,
}

impl Store {
    /// Save an element snapshot keyed by (url, key).
    pub fn save_element(&self, url: &str, key: &str, row: &ElementSnapshotRow) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO element_snapshots
             (url, key, tag, attrs, text_preview, ancestor_path, sibling_tags,
              position_in_parent, parent_tag, parent_attrs, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                url, key, row.tag,
                row.attrs.to_string(),
                row.text_preview,
                row.ancestor_path.to_string(),
                row.sibling_tags.to_string(),
                row.position_in_parent,
                row.parent_tag,
                row.parent_attrs.to_string(),
                row.captured_at,
            ],
        ).map_err(|e| WispError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Load an element snapshot by (url, key).
    pub fn load_element(&self, url: &str, key: &str) -> Result<Option<ElementSnapshotRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT tag, attrs, text_preview, ancestor_path, sibling_tags,
                    position_in_parent, parent_tag, parent_attrs, captured_at
             FROM element_snapshots WHERE url = ?1 AND key = ?2"
        ).map_err(|e| WispError::Storage(e.to_string()))?;

        let mut rows = stmt.query(params![url, key])
            .map_err(|e| WispError::Storage(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| WispError::Storage(e.to_string()))? {
            let tag: String = row.get(0).map_err(|e| WispError::Storage(e.to_string()))?;
            let attrs_str: String = row.get(1).map_err(|e| WispError::Storage(e.to_string()))?;
            let text_preview: String = row.get(2).map_err(|e| WispError::Storage(e.to_string()))?;
            let ancestor_path_str: String = row.get(3).map_err(|e| WispError::Storage(e.to_string()))?;
            let sibling_tags_str: String = row.get(4).map_err(|e| WispError::Storage(e.to_string()))?;
            let position_in_parent: i64 = row.get(5).map_err(|e| WispError::Storage(e.to_string()))?;
            let parent_tag: String = row.get(6).map_err(|e| WispError::Storage(e.to_string()))?;
            let parent_attrs_str: String = row.get(7).map_err(|e| WispError::Storage(e.to_string()))?;
            let captured_at: i64 = row.get(8).map_err(|e| WispError::Storage(e.to_string()))?;

            Ok(Some(ElementSnapshotRow {
                tag,
                attrs: serde_json::from_str(&attrs_str).unwrap_or(serde_json::json!({})),
                text_preview,
                ancestor_path: serde_json::from_str(&ancestor_path_str).unwrap_or(serde_json::json!([])),
                sibling_tags: serde_json::from_str(&sibling_tags_str).unwrap_or(serde_json::json!([])),
                position_in_parent,
                parent_tag,
                parent_attrs: serde_json::from_str(&parent_attrs_str).unwrap_or(serde_json::json!({})),
                captured_at,
            }))
        } else {
            Ok(None)
        }
    }
}
