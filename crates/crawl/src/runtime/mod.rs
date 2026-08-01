//! 运行时组件：会话池、自适应缩放、robots、缓存、控制。

pub mod autoscale;
pub mod control;
pub mod items;
pub mod output;
pub mod robots;

pub use control::EngineControl;
pub use items::{Items, JsonlWriter};
pub use robots::RobotsCache;
