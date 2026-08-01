//! 调度：URL 队列、停止条件。

pub mod scheduler;
pub mod stop;

pub use scheduler::Scheduler;
pub use stop::{
    pages_by_callback, FnStopCondition, MaxErrors, MaxItems, MaxPages, MaxPagesByCallback,
    NeverStop, StopCondition, StopContext, Timeout,
};
