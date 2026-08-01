//! Engine 运行时：Engine 结构体 + EngineBuilder + run_inner 流驱动。

mod builder;
mod config;
mod engine;
mod guard;
mod lifecycle;
mod setup;
mod work;

#[cfg(test)]
mod tests;

pub use builder::EngineBuilder;
pub use config::EngineConfig;
pub use engine::Engine;
pub(crate) use guard::RunGuard;
pub(crate) use work::{build_final_stats, run_stream_driver, run_work_loop};
