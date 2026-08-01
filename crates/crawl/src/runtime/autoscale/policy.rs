//! 扩容/缩容决策。

use std::sync::atomic::Ordering;
use std::time::Instant;

use super::AutoscaledPool;
use crate::observability::events::{EngineEvent, EventBus};

impl AutoscaledPool {
    pub(super) async fn maybe_scale_down(
        &self,
        error_rate: f64,
        saturation: f64,
        event_bus: Option<&EventBus>,
    ) {
        if error_rate <= self.config.error_rate_threshold
            && saturation >= self.config.cpu_threshold_up
        {
            return;
        }
        let now = Instant::now();
        let last_down = *self.last_scale_down.lock();
        if now.duration_since(last_down) < self.config.scale_down_interval {
            return;
        }
        let current = self.current.load(Ordering::SeqCst);
        let new_val = current
            .saturating_sub(self.config.step_down)
            .max(self.min_concurrency);
        if new_val >= current {
            return;
        }
        self.current.store(new_val, Ordering::SeqCst);
        *self.last_scale_down.lock() = now;
        tracing::debug!("Autoscale down (idle/err): {} -> {}", current, new_val);
        if let Some(bus) = event_bus {
            bus.emit(EngineEvent::ConcurrencyChanged {
                old: current,
                new: new_val,
            })
            .await;
        }
        self.notify_work();
    }

    pub(super) async fn maybe_scale_up(
        &self,
        error_rate: f64,
        saturation: f64,
        event_bus: Option<&EventBus>,
    ) {
        if saturation <= self.config.cpu_threshold_down
            || error_rate >= self.config.error_rate_threshold * 0.5
        {
            return;
        }
        let now = Instant::now();
        let last_up = *self.last_scale_up.lock();
        if now.duration_since(last_up) < self.config.scale_up_interval {
            return;
        }
        let current = self.current.load(Ordering::SeqCst);
        let new_val = (current + self.config.step_up).min(self.max_concurrency);
        if new_val <= current {
            return;
        }
        self.current.store(new_val, Ordering::SeqCst);
        *self.last_scale_up.lock() = now;
        tracing::debug!("Autoscale up (saturated): {} -> {}", current, new_val);
        if let Some(bus) = event_bus {
            bus.emit(EngineEvent::ConcurrencyChanged {
                old: current,
                new: new_val,
            })
            .await;
        }
        self.notify_work();
    }
}
