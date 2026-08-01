//! 事件驱动生命周期 — 细粒度内部事件总线。
//!
//! 借鉴 Crawlee EventManager + Scrapy Signals 设计：
//! 关键路径（fetch 完成、item 产出、错误、Auto 升级）emit 事件，
//! 用户可注册监听器实现监控、日志、指标采集、告警。
//!
//! # 零成本原则
//!
//! 无 listener 时 emit 为 no-op（仅检查 Vec 是否为空）。
//!
//! # 与 CrawlEvent 关系
//!
//! `CrawlEvent` 保留作为 `run_stream` 的外部接口。
//! `EngineEvent` 是更细粒度的内部事件总线。
//! 可通过一个 listener 将 EngineEvent 桥接到 CrawlEvent channel。

mod bus;
mod event;
mod listener;
mod metrics;

#[cfg(test)]
mod tests;

pub use bus::EventBus;
pub use event::EngineEvent;
pub use listener::{logging_listener, metrics_listener, EventListener};
pub use metrics::Metrics;
