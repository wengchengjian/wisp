//! 调度：URL 队列、停止条件。

pub mod scheduler;
pub mod stop;

pub use scheduler::Scheduler;
pub use stop::{
    FnStopCondition, MaxErrors, MaxItems, MaxPages, NeverStop, StopCondition, StopContext, Timeout,
};
