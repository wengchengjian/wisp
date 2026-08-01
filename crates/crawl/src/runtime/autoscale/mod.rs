//! 自适应并发池 — 根据池饱和度动态调整并发数。
//!
//! 借鉴 Crawlee AutoscaledPool 设计：定期采样饱和度（in_flight / current）
//! 与错误率，在池饱和（需求旺盛）时扩容、在池空闲或错误率高时缩容。
//!
//! # 集成
//!
//! `EngineBuilder` 新增 `.autoscale(min, max)` 选项。
//! 启用后主循环的 `buffer_unordered` 改为动态 semaphore。

mod config;
mod metrics;
mod policy;
mod worker;

#[cfg(test)]
mod tests;

pub use config::AutoscaleConfig;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

/// 自适应并发池。
///
/// 通过后台 task 定期采样系统指标，动态调整允许的并发数。
/// 主循环通过 `current_concurrency()` 获取当前限制。
///
/// ND-004-CORR/ND-007-PERF：调整并发时通过 `work_notify` 唤醒主循环，
/// 避免主循环 10ms timeout 轮询。
pub struct AutoscaledPool {
    min_concurrency: usize,
    max_concurrency: usize,
    current: Arc<AtomicUsize>,
    config: AutoscaleConfig,
    last_scale_up: Arc<Mutex<Instant>>,
    last_scale_down: Arc<Mutex<Instant>>,
    /// ND-004-CORR：主循环的 Notify，扩容时唤醒等待的派发循环。
    work_notify: Arc<Mutex<Option<Arc<tokio::sync::Notify>>>>,
}

impl AutoscaledPool {
    /// 创建自适应并发池。
    pub fn new(
        min_concurrency: usize,
        max_concurrency: usize,
        config: AutoscaleConfig,
    ) -> Arc<Self> {
        let initial = min_concurrency.max(1);
        Arc::new(Self {
            min_concurrency: min_concurrency.max(1),
            max_concurrency: max_concurrency.max(initial),
            current: Arc::new(AtomicUsize::new(initial)),
            config,
            last_scale_up: Arc::new(Mutex::new(Instant::now())),
            last_scale_down: Arc::new(Mutex::new(Instant::now())),
            work_notify: Arc::new(Mutex::new(None)),
        })
    }

    /// 获取当前允许的并发数（主循环使用）。
    pub fn current_concurrency(&self) -> usize {
        self.current.load(Ordering::SeqCst)
    }

    /// 获取最大并发数上限（主循环用作 buffer_unordered 的 ceiling）。
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// ND-004-CORR：注入主循环的 Notify，扩容时唤醒等待的派发循环。
    ///
    /// 主循环在创建 pool 后调用此方法注入 Notify 引用。
    /// autoscaler 扩容时会调用 `notify_one()`，避免主循环 10ms 轮询。
    pub fn set_work_notify(&self, notify: Arc<tokio::sync::Notify>) {
        *self.work_notify.lock() = Some(notify);
    }

    /// 扩容/缩容后唤醒主循环（若已注入 Notify）。
    fn notify_work(&self) {
        if let Some(ref n) = *self.work_notify.lock() {
            n.notify_one();
        }
    }
}
