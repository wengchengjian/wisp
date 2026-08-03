//! EngineBuilder 构造器子模块。

mod build;
mod methods;

use std::sync::Arc;

use super::Engine;
use super::config::EngineConfig;
use crate::middleware::{ItemPipeline, Middleware, UaRotationMiddleware};
use crate::observability::events::{EventBus, EventCallback};
use crate::runtime::autoscale::AutoscaledPool;
use wisp_fetcher::FetchClient;
use wisp_storage::Store;

/// Engine 构造器（Builder 模式）。
///
/// 用户配置集中在 `config`，运行时资源单独持有。
pub struct EngineBuilder {
    pub(crate) config: EngineConfig,
    pub(crate) fetch_client: Option<Arc<FetchClient>>,
    pub(crate) cache_store: Option<Arc<dyn Store>>,
    pub(crate) checkpoint_store: Option<Arc<dyn Store>>,
    pub(crate) autoscale: Option<Arc<AutoscaledPool>>,
    pub(crate) event_bus: EventBus,
    pub(crate) ua_middleware: Option<Arc<UaRotationMiddleware>>,
    pub(crate) custom_middlewares: Vec<Arc<dyn Middleware>>,
    pub(crate) pipelines: Vec<Arc<dyn ItemPipeline>>,
}

impl Engine {
    /// 创建 Engine builder（纯基础设施构造器）。
    pub fn infra() -> EngineBuilder {
        EngineBuilder {
            config: EngineConfig::default(),
            fetch_client: None,
            cache_store: None,
            checkpoint_store: None,
            autoscale: None,
            event_bus: EventBus::new(),
            ua_middleware: None,
            custom_middlewares: Vec::new(),
            pipelines: Vec::new(),
        }
    }
}
