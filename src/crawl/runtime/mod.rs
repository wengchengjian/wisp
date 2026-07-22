//! 运行时组件：会话池、自适应缩放、robots、缓存、控制。

pub mod session_pool;
pub mod autoscale;
pub mod robots;
pub mod items;
pub mod output;
pub mod request_cache;
pub mod cache;
pub mod control;

pub use items::{Items, JsonlWriter};
pub use request_cache::RequestCache;
pub use robots::RobotsCache;
pub use control::EngineControl;
