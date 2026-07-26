//! 浏览器抓取策略实现。

pub mod dynamic;
pub mod stealth;

pub use dynamic::DynamicStrategy;
pub use stealth::StealthStrategy;
