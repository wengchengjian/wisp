//! Engine runtime resources, kept separate from EngineConfig.

use std::sync::Arc;

use crate::middleware::{ItemPipeline, Middleware, UaRotationMiddleware};
use crate::observability::events::EventBus;
use crate::runtime::autoscale::AutoscaledPool;
use wisp_fetcher::FetchClient;
use wisp_storage::Store;

/// 运行时资源：可共享、可替换，但不属于用户配置。
#[derive(Clone)]
pub(crate) struct EngineRuntime {
    /// 共享 FetchClient（HTTP 连接池 + BrowserPool，跨 Spider 复用）。
    pub fetch_client: Arc<FetchClient>,
    /// 响应缓存存储（可选）。
    pub cache_store: Option<Arc<dyn Store>>,
    /// checkpoint 存储（可选）。
    pub checkpoint_store: Option<Arc<dyn Store>>,
    /// 自适应并发池（可选）。
    pub autoscale: Option<Arc<AutoscaledPool>>,
    /// 引擎内部事件总线。
    pub event_bus: Arc<EventBus>,
    /// UA 轮换中间件实例（可选）。
    pub ua_middleware: Option<Arc<UaRotationMiddleware>>,
    /// 用户自定义 Engine 级中间件。
    pub custom_middlewares: Vec<Arc<dyn Middleware>>,
    /// 共享 item pipeline。
    pub pipelines: Vec<Arc<dyn ItemPipeline>>,
}
