//! 事件驱动生命周期 — 单一事件流。
//!
//! `CrawlEvent` 是唯一事件类型：Engine emit 一次，所有 listener 与
//! `subscribe()` 订阅者都会收到同一份事实。

mod bus;
mod listener;
mod metrics;

#[cfg(test)]
mod tests;

pub use bus::{EventBus, Subscription};
pub use listener::{EventCallback, EventListener, logging_listener, metrics_listener};
pub use metrics::Metrics;
