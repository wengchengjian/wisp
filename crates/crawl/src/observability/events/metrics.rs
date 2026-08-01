//! 简单指标收集器。

/// 简单指标收集器。
#[derive(Debug, Default)]
pub struct Metrics {
    /// 响应数。
    pub responses: std::sync::atomic::AtomicUsize,
    /// Item 数。
    pub items: std::sync::atomic::AtomicUsize,
    /// 错误数。
    pub errors: std::sync::atomic::AtomicUsize,
    /// 缓存命中数。
    pub cache_hits: std::sync::atomic::AtomicUsize,
    /// 总耗时（毫秒）。
    pub total_elapsed_ms: std::sync::atomic::AtomicU64,
}

impl Metrics {
    /// 创建新的指标收集器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 平均响应时间（毫秒）。
    pub fn avg_response_ms(&self) -> u64 {
        let responses = self.responses.load(std::sync::atomic::Ordering::Relaxed);
        if responses == 0 {
            return 0;
        }
        self.total_elapsed_ms
            .load(std::sync::atomic::Ordering::Relaxed)
            / responses as u64
    }
}
