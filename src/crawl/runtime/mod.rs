//! 运行时组件：自适应缩放、robots、缓存、控制。

pub mod autoscale;
pub mod robots;
pub mod items;
pub mod output;
pub mod control;

pub use items::{Items, JsonlWriter};
pub use robots::RobotsCache;
pub use control::EngineControl;
