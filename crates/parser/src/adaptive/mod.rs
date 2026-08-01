//! Adaptive element relocation based on similarity matching.
//!
//! Port of Python Scrapling's adaptive relocation: capture element snapshots,
//! persist to SQLite, and relocate when site markup changes.
//!
//! ARCH: 本模块保持纯函数/值对象语义，持久化由 `crawl::adaptive::AdaptiveTracker` 承担。

mod helpers;
mod relocate;
mod score;
mod snapshot;

pub use relocate::relocate_with_snapshot;
pub use score::{similarity, DEFAULT_TOLERANCE};
pub use snapshot::ElementSnapshot;
