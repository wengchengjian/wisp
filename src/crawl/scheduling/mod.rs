//! 调度：URL 队列、停止条件、定时任务。

pub mod scheduler;
pub mod stop;
pub mod cron;

pub use scheduler::Scheduler;
pub use stop::{StopCondition, StopContext, MaxPages, MaxItems, MaxErrors, Timeout, NeverStop, FnStopCondition};
pub use cron::CronExpr;
