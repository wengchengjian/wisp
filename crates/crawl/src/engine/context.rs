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

/// 单次 run 状态：跨 task 共享的调度资源与 per-run 可变状态。
pub(crate) struct EngineState {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<Request>>>,
    pub work_notify: Arc<tokio::sync::Notify>,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    pub cf_domain_locks: Arc<dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub spiders: Vec<Arc<dyn Spider>>,
    pub all_stats: Vec<Arc<SpiderStats>>,
    pub items: Arc<Mutex<Vec<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    pub global_in_flight: Arc<AtomicUsize>,
    pub in_flight_requests: Arc<Mutex<HashMap<String, Vec<Request>>>>,
}

impl EngineState {
    /// 返回请求应路由到的 Spider 索引；`Request.spider` 优先，否则按 callback 归属查找。
    pub fn spider_index_for(&self, req: &Request) -> Option<usize> {
        if let Some(name) = req.spider.as_deref() {
            return self.spiders.iter().position(|s| s.name() == name);
        }
        // callback 是跨 Spider 的路由键（小说爬虫 home→detail→chapter）；
        // 只有唯一 Spider 接受时才按 callback 路由，多个 Spider 同名 handler 视为歧义。
        let matches: Vec<usize> = self
            .spiders
            .iter()
            .enumerate()
            .filter(|(_, s)| s.accepts_callback(req.callback.as_deref()))
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [idx] => Some(*idx),
            [] => None,
            _ => {
                tracing::warn!(
                    "callback {:?} 被多个 Spider 接受 ({} 个)，未绑定 spider 的请求无法路由: url={}",
                    req.callback,
                    matches.len(),
                    sanitize_url(&req.url)
                );
                None
            }
        }
    }

    pub fn spider_for(&self, req: &Request) -> Option<Arc<dyn Spider>> {
        self.spider_index_for(req)
            .map(|i| Arc::clone(&self.spiders[i]))
    }

    pub fn stats_for(&self, req: &Request) -> Option<Arc<SpiderStats>> {
        self.spider_index_for(req)
            .map(|i| Arc::clone(&self.all_stats[i]))
    }
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
