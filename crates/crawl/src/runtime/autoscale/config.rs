//! 自适应并发池配置。

use std::time::Duration;

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
