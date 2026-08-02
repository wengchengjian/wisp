//! Engine 子模块：context。

use super::*;
use crate::runner::EngineConfig as UserEngineConfig;

// === EngineContext: 打包所有共享状态 ===

/// Engine 运行时上下文（单 Spider），由三层子结构组成。
///
/// - `config`: 只读配置（从 Spider 提取，run 期间不变）
/// - `shared`: 跨 task 共享的可变状态
/// - `state`: per-run 可变状态
pub(crate) struct EngineContext {
    pub config: EngineConfig,
    pub shared: EngineShared,
    pub state: EngineState,
}

/// 只读配置（从 Spider 提取，run 期间不变）。
pub(crate) struct EngineConfig {
    /// 唯一用户配置源。
    pub user: UserEngineConfig,
    /// 共享 FetchClient。
    pub client: Arc<wisp_fetcher::FetchClient>,
    /// checkpoint 存储（可选）。
    pub checkpoint_store: Option<Arc<dyn wisp_storage::Store>>,
}

/// 跨 task 共享的可变状态。
pub(crate) struct EngineShared {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<Request>>>,
    pub control: Arc<control::EngineControl>,
    pub work_notify: Arc<tokio::sync::Notify>,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub event_bus: Arc<EventBus>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    /// 域名级 CF 挑战锁：防止初始并发请求全部走浏览器。
    /// 第一个请求获取锁并解决 CF，其他请求等待后复用 cookie。
    pub cf_domain_locks: Arc<dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// per-run 可变状态。
pub(crate) struct EngineState {
    pub spiders: Vec<Arc<dyn Spider>>,
    pub all_stats: Vec<Arc<SpiderStats>>,
    pub items: Arc<Mutex<Vec<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    pub global_in_flight: Arc<AtomicUsize>,
    /// 各 Spider 当前 in-flight 请求（key=spider name，checkpoint 持久化用）。
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
        fetch_mode: ctx.config.user.fetch_mode,
        max_concurrent: ctx.config.user.max_concurrent,
        max_pages: ctx.config.user.max_pages,
        obey_robots: ctx.config.user.obey_robots,
        pages_crawled: stats.pages.load(Ordering::SeqCst),
        errors: stats.errors.load(Ordering::SeqCst),
    }
}

/// 同步记录状态码计数（DashMap entry 原子累加，无 await）。
#[doc(hidden)]
pub fn record_status(stats: &Arc<SpiderStats>, status: u16) {
    stats
        .status_codes
        .entry(status)
        .and_modify(|c| {
            c.fetch_add(1, Ordering::Relaxed);
        })
        .or_insert(AtomicUsize::new(1));
}

/// 从单个 SpiderStats 构造 CrawlStats 快照。
pub(crate) fn snapshot_stats_for(
    stats: &Arc<SpiderStats>,
    status_codes: HashMap<u16, usize>,
) -> CrawlStats {
    CrawlStats {
        items_scraped: stats.items.load(Ordering::SeqCst),
        pages_crawled: stats.pages.load(Ordering::SeqCst),
        errors: stats.errors.load(Ordering::SeqCst),
        duration: stats.elapsed(),
        blocked_requests: stats.blocked.load(Ordering::SeqCst),
        retry_count: stats.retries.load(Ordering::SeqCst),
        status_code_counts: status_codes,
        offsite_requests_count: stats.offsite.load(Ordering::SeqCst),
        cache_hits: stats.cache_hits.load(Ordering::SeqCst),
        ..Default::default()
    }
}
