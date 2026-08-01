//! 并发池饱和度与错误率采样。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::AutoscaledPool;
use crate::observability::stats::SpiderStats;

impl AutoscaledPool {
    pub(super) fn sample_metrics(
        &self,
        stats: &[Arc<SpiderStats>],
        last_pages: usize,
        last_errors: usize,
    ) -> (f64, f64, usize, usize) {
        let current_pages = stats
            .iter()
            .map(|s| s.pages.load(Ordering::SeqCst))
            .sum::<usize>();
        let current_errors = stats
            .iter()
            .map(|s| s.errors.load(Ordering::SeqCst))
            .sum::<usize>();
        let pages_delta = current_pages.saturating_sub(last_pages);
        let errors_delta = current_errors.saturating_sub(last_errors);
        let error_rate = if pages_delta + errors_delta > 0 {
            errors_delta as f64 / (pages_delta + errors_delta) as f64
        } else {
            0.0
        };
        let in_flight = stats
            .iter()
            .map(|s| s.in_flight.load(Ordering::SeqCst))
            .sum::<usize>();
        let current = self.current.load(Ordering::SeqCst);
        let saturation = if current > 0 {
            in_flight as f64 / current as f64
        } else {
            0.0
        };
        (error_rate, saturation, current_pages, current_errors)
    }
}
