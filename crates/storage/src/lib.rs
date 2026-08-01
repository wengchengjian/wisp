//! 统一存储层：可插拔的持久化后端 trait + Memory/File/SQLite 实现。
//!
//! 三类用途（通过自由函数实现，trait 仅提供底层 KV 原语）：
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

mod functions;
mod models;
mod store;

pub use functions::{
    delete_checkpoint, delete_response, load_checkpoint, load_element, load_response,
    save_checkpoint, save_element, save_response,
};
pub use models::{CachedResponse, ElementSnapshotRow};
pub use store::Store;

#[cfg(test)]
mod tests;
