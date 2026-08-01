//! Engine 公开配置快照。

use std::time::Duration;

use wisp_fetcher::{FetchClientConfig, FetchMode};

/// Engine 公开配置快照（ARCH: master 公开 `EngineConfig` 的轻量移植）。
///
/// 只读视图，由 [`Engine::config`] 生成；内部实现仍以 Engine 字段为事实源。
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// 共享 FetchClient 传输配置（HTTP/浏览器/代理等单一事实源）。
    pub transport: FetchClientConfig,
    /// 最大并发数。
    pub max_concurrent: usize,
    /// 最大爬取页数（引擎级兜底）。
    pub max_pages: usize,
    /// 抓取模式。
    pub fetch_mode: FetchMode,
    /// 是否遵守 robots.txt。
    pub obey_robots: bool,
    /// 网络错误重试上限。
    pub max_retries: u32,
    /// 响应中间件 Refetch 最大轮数。
    pub max_refetch_rounds: usize,
    /// 下载延迟。
    pub download_delay: Duration,
    /// 固定请求头。
    pub headers: Vec<(String, String)>,
    /// 是否启用 UA 轮换。
    pub ua_rotation: bool,
    /// 是否启用 Cookie Challenge。
    pub cookie_challenge: bool,
    /// 是否启用 DynamicUpgrade。
    pub dynamic_upgrade: bool,
    /// Auto 模式 URL 规则。
    pub auto_rules: Vec<(String, FetchMode)>,
    /// 是否启用响应缓存。
    pub cache_enabled: bool,
    /// 是否启用 checkpoint 持久化。
    pub checkpoint_enabled: bool,
    /// checkpoint 保存间隔（页数）。
    pub checkpoint_interval: usize,
}
