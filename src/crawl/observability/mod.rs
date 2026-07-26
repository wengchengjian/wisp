//! 可观测性：事件总线、统计、状态。

pub mod events;
pub mod state;
pub mod stats;

pub use state::CrawlState;
pub use stats::SpiderStats;
