//! 自适应解析跟踪器 — 持久化元素快照。
//!
//! ARCH: 从 `parser/adaptive.rs` 迁移。parser 只产出 `ElementSnapshot` 值对象，
//! 持久化职责由 `AdaptiveTracker` 承担，消除 parser 对 storage 的依赖。

mod convert;
mod tracker;

#[cfg(test)]
mod tests;

pub use tracker::AdaptiveTracker;
