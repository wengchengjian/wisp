//! 浏览器抓取策略实现。

#[cfg(feature = "browser")]
pub mod dynamic;
#[cfg(feature = "stealth")]
pub mod stealth;

#[cfg(feature = "browser")]
pub use dynamic::DynamicStrategy;
#[cfg(feature = "stealth")]
pub use stealth::StealthStrategy;
