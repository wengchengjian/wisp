//! 后台 autoscaler 采样循环。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::AutoscaledPool;
use crate::observability::events::EventBus;
use crate::observability::stats::SpiderStats;

impl AutoscaledPool {
    /// 后台 autoscaler task：定期采样系统指标，调整 desired concurrency。
    ///
    /// 应在 `run_inner` 中 spawn 此 task，爬取结束后 abort。
    pub async fn run_autoscaler(
        self: &Arc<Self>,
        stats: Vec<Arc<SpiderStats>>,
        event_bus: Option<Arc<EventBus>>,
    ) {
        let mut interval = tokio::time::interval(self.config.sample_interval);
        let mut last_pages = stats
            .iter()
            .map(|s| s.pages.load(Ordering::SeqCst))
            .sum::<usize>();
        let mut last_errors = stats
            .iter()
            .map(|s| s.errors.load(Ordering::SeqCst))
            .sum::<usize>();

        loop {
            interval.tick().await;
            let (error_rate, saturation, current_pages, current_errors) =
                self.sample_metrics(&stats, last_pages, last_errors);
            last_pages = current_pages;
            last_errors = current_errors;
            self.maybe_scale_down(error_rate, saturation, event_bus.as_deref())
                .await;
            self.maybe_scale_up(error_rate, saturation, event_bus.as_deref())
                .await;
        }
    }
}
