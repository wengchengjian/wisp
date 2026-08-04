//! Engine 子模块：context。

use super::*;

// === EngineContext: 打包单次 run 状态 ===

/// Engine 运行时上下文（单次 run），由配置、运行时资源和 run 状态组成。
///
/// - `config`: 唯一用户配置源
/// - `runtime`: Engine 长生命周期运行时资源
/// - `state`: 单次 run 的调度与可变状态
pub(crate) struct EngineContext {
    pub config: EngineConfig,
    pub runtime: EngineRuntime,
    pub state: EngineState,
}

/// 单次 run 输入草稿：由 run_inner_many 组装，build 时转为 EngineState。
pub(crate) struct EngineRunDraft {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<CrawlRequest>,
    pub follow_rx: tokio::sync::mpsc::UnboundedReceiver<CrawlRequest>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    pub robots_cache: Arc<crate::runtime::robots::RobotsCache>,
    pub spiders: Vec<Arc<dyn Spider>>,
    pub all_stats: Vec<Arc<SpiderStats>>,
}

/// 调度队列状态：跨 task 共享的调度资源。
///
/// `follow_tx`/`follow_rx` 是 follow 请求的生产/消费端，
/// `work_notify` 唤醒等待新 work 的 worker。
pub(crate) struct QueueState {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<CrawlRequest>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<CrawlRequest>>>,
    pub work_notify: Arc<tokio::sync::Notify>,
}

/// Spider 注册表：持有路由决策器 + 统计并行数组。
///
/// 路由逻辑委托给 `RequestRouter`（独立可测的深模块）；`router.spiders[i]`
/// 与 `all_stats[i]` 一一对应。路由优先级：
/// `Request.spider` 显式指定 > callback 唯一归属 > 歧义（warn + None）。
pub(crate) struct SpiderRegistry {
    pub router: super::router::RequestRouter,
    pub all_stats: Vec<Arc<SpiderStats>>,
}

impl SpiderRegistry {
    /// 创建注册表：路由决策器持有 spiders，stats 与之一一对应。
    pub fn new(spiders: Vec<Arc<dyn Spider>>, all_stats: Vec<Arc<SpiderStats>>) -> Self {
        Self {
            router: super::router::RequestRouter::new(spiders),
            all_stats,
        }
    }

    /// 返回请求应路由到的 Spider 索引（委托 `RequestRouter::route`）。
    pub fn spider_index_for(&self, req: &CrawlRequest) -> Option<usize> {
        self.router.route(req)
    }

    pub fn spider_for(&self, req: &CrawlRequest) -> Option<Arc<dyn Spider>> {
        self.router
            .route(req)
            .and_then(|i| self.router.get(i).map(Arc::clone))
    }

    pub fn stats_for(&self, req: &CrawlRequest) -> Option<Arc<SpiderStats>> {
        self.router
            .route(req)
            .map(|i| Arc::clone(&self.all_stats[i]))
    }

    /// 只读访问 Spider 列表（供遍历与索引对齐）。
    pub fn spiders(&self) -> &[Arc<dyn Spider>] {
        self.router.spiders()
    }
}

/// CF 域名锁映射：per-domain 互斥锁（stealth override 串行化）。
///
/// 容量上限与全局回退锁由 `fetch::page::auto_mode` 管理。
pub(crate) struct CfLockMap {
    pub locks: Arc<dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// Run 运行时状态：中止信号 + pipeline 错误 + 在途请求跟踪。
pub(crate) struct RunState {
    pub abort_flag: Arc<AtomicBool>,
    pub pipeline_error: Arc<Mutex<Option<wisp_core::error::WispError>>>,
    pub global_in_flight: Arc<AtomicUsize>,
    pub in_flight_requests: Arc<Mutex<HashMap<String, Vec<CrawlRequest>>>>,
}

/// 单次 run 状态：跨 task 共享的调度资源与 per-run 可变状态。
///
/// 按概念分组为子结构体：
/// - `queue`: 调度队列（sched + follow channel + work_notify）
/// - `middleware_chain`: 中间件链
/// - `rule_engine`: Auto 模式规则引擎
/// - `cf_locks`: CF 域名锁
/// - `spiders`: Spider 注册表（含路由方法）
/// - `run`: Run 状态（abort + error + in_flight）
pub(crate) struct EngineState {
    pub queue: QueueState,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    pub cf_locks: CfLockMap,
    pub spiders: SpiderRegistry,
    pub run: RunState,
}

pub(crate) fn build_crawl_context_for(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
) -> middleware::CrawlContext {
    middleware::CrawlContext {
        spider_name: spider.name().to_string(),
        fetch_mode: ctx.config.fetch_mode,
        max_concurrent: ctx.config.max_concurrent,
        max_pages: ctx.config.max_pages,
        obey_robots: ctx.config.obey_robots,
        pages_crawled: stats.pages.load(Ordering::SeqCst),
        errors: stats.errors.load(Ordering::SeqCst),
    }
}
