//! Engine 公开配置：builder 产出、Engine 持有的唯一配置源。

use std::time::Duration;

use wisp_fetcher::{FetchClientConfig, FetchMode};

/// Engine 配置。
///
/// 运行时资源（FetchClient、store、autoscale、event bus、middleware、pipeline）
/// 不进入本结构，由 `EngineRuntime` 持有。
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// 共享 FetchClient 传输配置（HTTP/浏览器/代理等单一事实源）。
    pub transport: FetchClientConfig,
    /// 最大并发数。
    pub max_concurrent: usize,
    /// 最大爬取页数（引擎级兜底）。`None` 表示不限制，由「队列空 + 停止条件」自然结束。
    pub max_pages: Option<usize>,
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

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            transport: FetchClientConfig::default(),
            max_concurrent: 8,
            max_pages: None,
            fetch_mode: FetchMode::Auto,
            obey_robots: true,
            max_retries: 3,
            max_refetch_rounds: 5,
            download_delay: Duration::ZERO,
            headers: Vec::new(),
            ua_rotation: false,
            cookie_challenge: false,
            dynamic_upgrade: false,
            auto_rules: Vec::new(),
            cache_enabled: false,
            checkpoint_enabled: false,
            checkpoint_interval: 100,
        }
    }
}
