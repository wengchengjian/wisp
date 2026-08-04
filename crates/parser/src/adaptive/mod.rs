//! Adaptive element relocation based on similarity matching.
//!
//! Port of Python Scrapling's adaptive relocation: capture element snapshots,
//! persist to SQLite, and relocate when site markup changes.
//!
//! ARCH: 本模块保持纯函数/值对象语义，外加 `AdaptiveTracker` 持久化跟踪器。

mod convert;
mod helpers;
mod relocate;
mod score;
mod snapshot;
mod tracker;

#[cfg(test)]
mod tests;

pub use convert::{row_to_snapshot, snapshot_to_row};
pub use relocate::relocate_with_snapshot;
pub use score::{DEFAULT_TOLERANCE, similarity};
pub use snapshot::ElementSnapshot;
pub use tracker::AdaptiveTracker;
