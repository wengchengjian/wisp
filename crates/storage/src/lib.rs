//! 统一存储层：可插拔的持久化后端 trait + Memory/File/SQLite 实现。
//!
//! 三类用途（作为 `Store` trait 的默认方法实现）：
//! - Checkpoint（断点续爬）：`save_checkpoint` / `load_checkpoint` / `delete_checkpoint`
//! - Element Snapshot（自适应定位）：`save_element` / `load_element`
//! - Response Cache（HTTP 响应缓存，带 per-entry TTL）：`save_response` / `load_response` / `delete_response`

#[cfg(feature = "sqlite")]
pub mod migrations;

mod file;
mod memory;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use file::FileStore;
pub use memory::MemoryStore;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;

mod models;
mod store;

pub use models::{CachedResponse, ElementSnapshotRow};
pub use store::{Store, open_store};

#[cfg(test)]
mod tests;
