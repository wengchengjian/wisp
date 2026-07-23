//! 自适应并发池 — 根据池饱和度动态调整并发数。
//!
//! 借鉴 Crawlee AutoscaledPool 设计：定期采样饱和度（in_flight / current）
//! 与错误率，在池饱和（需求旺盛）时扩容、在池空闲或错误率高时缩容。
//!
//! # 集成
//!
//! `EngineBuilder` 新增 `.autoscale(min, max)` 选项。
//! 启用后主循环的 `buffer_unordered` 改为动态 semaphore。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::crawl::observability::stats::SpiderStats;

/// 自适应并发池配置。
#[derive(Debug, Clone)]
pub struct AutoscaleConfig {
    /// 扩容冷却时间（默认 5s）
    pub scale_up_interval: Duration,
    /// 缩容冷却时间（默认 2s）
    pub scale_down_interval: Duration,
    /// 饱和度低于此值时缩容（默认 0.7，池空闲回收资源）
    pub cpu_threshold_up: f64,
    /// 饱和度高于此值时扩容（默认 0.9，需求旺盛加容量）
    pub cpu_threshold_down: f64,
    /// 错误率高于此值时缩容（默认 0.2）
    pub error_rate_threshold: f64,
    /// 每次扩容步长（默认 2）
    pub step_up: usize,
    /// 每次缩容步长（默认 4）
    pub step_down: usize,
    /// 采样间隔（默认 1s）
    pub sample_interval: Duration,
}

impl Default for AutoscaleConfig {
    fn default() -> Self {
        Self {
            scale_up_interval: Duration::from_secs(5),
            scale_down_interval: Duration::from_secs(2),
            cpu_threshold_up: 0.7,
            cpu_threshold_down: 0.9,
            error_rate_threshold: 0.2,
            step_up: 2,
            step_down: 4,
            sample_interval: Duration::from_secs(1),
        }
    }
}

/// 自适应并发池。
///
/// 通过后台 task 定期采样系统指标，动态调整允许的并发数。
/// 主循环通过 `current_concurrency()` 获取当前限制。
pub struct AutoscaledPool {
    min_concurrency: usize,
    max_concurrency: usize,
    current: Arc<AtomicUsize>,
    config: AutoscaleConfig,
    last_scale_up: Arc<std::sync::Mutex<Instant>>,
    last_scale_down: Arc<std::sync::Mutex<Instant>>,
}

impl AutoscaledPool {
    /// 创建自适应并发池。
    pub fn new(min_concurrency: usize, max_concurrency: usize, config: AutoscaleConfig) -> Arc<Self> {
        let initial = min_concurrency.max(1);
        Arc::new(Self {
            min_concurrency: min_concurrency.max(1),
            max_concurrency: max_concurrency.max(initial),
            current: Arc::new(AtomicUsize::new(initial)),
            config,
            last_scale_up: Arc::new(std::sync::Mutex::new(Instant::now())),
            last_scale_down: Arc::new(std::sync::Mutex::new(Instant::now())),
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

    /// 后台 autoscaler task：定期采样系统指标，调整 desired concurrency。
    ///
    /// 应在 `run_inner` 中 spawn 此 task，爬取结束后 abort。
    pub async fn run_autoscaler(self: &Arc<Self>, stats: Arc<SpiderStats>) {
        let mut interval = tokio::time::interval(self.config.sample_interval);
        let mut last_pages = stats.pages.load(Ordering::SeqCst);
        let mut last_errors = stats.errors.load(Ordering::SeqCst);

        loop {
            interval.tick().await;

            let current_pages = stats.pages.load(Ordering::SeqCst);
            let current_errors = stats.errors.load(Ordering::SeqCst);

            // 计算采样间隔内的错误率
            let pages_delta = current_pages.saturating_sub(last_pages);
            let errors_delta = current_errors.saturating_sub(last_errors);
            last_pages = current_pages;
            last_errors = current_errors;

            let error_rate = if pages_delta + errors_delta > 0 {
                errors_delta as f64 / (pages_delta + errors_delta) as f64
            } else {
                0.0
            };

            // 饱和度 = in_flight / current（I/O 爬虫：高饱和=需求旺盛应扩容，低饱和=空闲应缩容）
            let in_flight = stats.in_flight.load(Ordering::SeqCst);
            let current = self.current.load(Ordering::SeqCst);
            let saturation = if current > 0 {
                in_flight as f64 / current as f64
            } else {
                0.0
            };

            let now = Instant::now();

            // 缩容条件：错误率过高 或 饱和度低（空闲，回收资源）
            if error_rate > self.config.error_rate_threshold || saturation < self.config.cpu_threshold_up {
                let last_down = *self.last_scale_down.lock().unwrap();
                if now.duration_since(last_down) >= self.config.scale_down_interval {
                    let new_val = current.saturating_sub(self.config.step_down).max(self.min_concurrency);
                    if new_val < current {
                        self.current.store(new_val, Ordering::SeqCst);
                        *self.last_scale_down.lock().unwrap() = now;
                        tracing::debug!("Autoscale down (idle/err): {} -> {}", current, new_val);
                    }
                }
            }
            // 扩容条件：饱和度高（需求旺盛，加容量）且错误率可控
            else if saturation > self.config.cpu_threshold_down && error_rate < self.config.error_rate_threshold * 0.5 {
                let last_up = *self.last_scale_up.lock().unwrap();
                if now.duration_since(last_up) >= self.config.scale_up_interval {
                    let new_val = (current + self.config.step_up).min(self.max_concurrency);
                    if new_val > current {
                        self.current.store(new_val, Ordering::SeqCst);
                        *self.last_scale_up.lock().unwrap() = now;
                        tracing::debug!("Autoscale up (saturated): {} -> {}", current, new_val);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autoscaled_pool_creation() {
        let pool = AutoscaledPool::new(2, 16, AutoscaleConfig::default());
        assert_eq!(pool.current_concurrency(), 2);
    }

    #[test]
    fn test_autoscaled_pool_min_max() {
        let pool = AutoscaledPool::new(0, 0, AutoscaleConfig::default());
        // min 至少为 1
        assert_eq!(pool.current_concurrency(), 1);
    }

    #[tokio::test]
    async fn test_autoscaler_runs() {
        let pool = AutoscaledPool::new(2, 8, AutoscaleConfig {
            sample_interval: Duration::from_millis(50),
            ..Default::default()
        });
        let stats = Arc::new(SpiderStats::new());
        let pool_clone = Arc::clone(&pool);
        let stats_clone = Arc::clone(&stats);
        let handle = tokio::spawn(async move {
            pool_clone.run_autoscaler(stats_clone).await;
        });
        // 运行一小段时间后 abort
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();
        // 并发数应仍在合理范围内
        let current = pool.current_concurrency();
        assert!(current >= 2 && current <= 8);
    }
}
