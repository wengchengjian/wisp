//! EngineBuilder 构造器子模块。

mod build;
mod methods;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use super::Engine;
use crate::control;
use crate::observability::events::{EventBus, EventListener};
use wisp_core::error::Result;
use wisp_fetcher::{FetchClient, FetchClientConfig};

/// Engine 构造器（Builder 模式）。
pub struct EngineBuilder {
    fetch_client: Option<Arc<FetchClient>>,
    fetch_client_config: FetchClientConfig,
    max_concurrent: usize,
    max_pages: usize,
    max_refetch_rounds: usize,
    cache_store: Option<Arc<dyn wisp_storage::Store>>,
    checkpoint_store: Option<Arc<dyn wisp_storage::Store>>,
    checkpoint_interval: usize,
    autoscale: Option<Arc<crate::runtime::autoscale::AutoscaledPool>>,
    event_bus: EventBus,
    // === 引擎配置（ND-031-ARCH） ===
    fetch_mode: wisp_fetcher::FetchMode,
    obey_robots: bool,
    max_retries: u32,
    download_delay: Duration,
    headers: Vec<(String, String)>,
    ua_middleware: Option<Arc<crate::middleware::UaRotationMiddleware>>,
    cookie_challenge: bool,
    dynamic_upgrade: bool,
    custom_middlewares: Vec<Arc<dyn crate::middleware::Middleware>>,
    pipelines: Vec<Arc<dyn crate::middleware::ItemPipeline>>,
    auto_rules: Vec<(String, wisp_fetcher::FetchMode)>,
}

impl Engine {
    /// 创建 Engine builder（纯基础设施构造器）。
    ///
    /// 替代原 `Engine::new(spider)` / `Engine::spiders(vec)` / `Engine::builder(spider)`。
    /// Engine 不再持有 Spider，长期持有共享底层资源。
    pub fn infra() -> EngineBuilder {
        EngineBuilder {
            fetch_client: None,
            fetch_client_config: FetchClientConfig::default(),
            max_concurrent: 8,
            max_pages: 1000,
            max_refetch_rounds: 5,
            // 响应缓存默认关闭：单次爬取不需要回放，避免每页序列化/反序列化开销。
            cache_store: None,
            checkpoint_store: None,
            checkpoint_interval: 100,
            autoscale: None,
            event_bus: EventBus::new(),
            // 引擎配置默认值（ND-031-ARCH：原 Spider trait 默认值）
            fetch_mode: wisp_fetcher::FetchMode::Auto,
            obey_robots: true,
            max_retries: 3,
            download_delay: Duration::ZERO,
            headers: Vec::new(),
            ua_middleware: None,
            cookie_challenge: false,
            dynamic_upgrade: false,
            custom_middlewares: Vec::new(),
            pipelines: Vec::new(),
            auto_rules: Vec::new(),
        }
    }
}
