//! Engine 实现 - 从 mod.rs 拆分，降低圈复杂度。
//!
//! 核心拆解：
//! - `EngineContext` 打包所有共享状态（替代 20+ 个 Arc 变量传递）
//! - `process_request()` 处理单个请求（替代 200 行嵌套闭包）
//! - `fetch_dispatch()` 抓取分发循环（transport 级重试 fallback）
//! - `auto_upgrade_check()` Auto 模式升级检查
//!
//! Task 3 重构：EngineContext 单 Spider 化（删除 Vec + 路由），process_request
//! 调 `spider.handle()` 而非 `spider.parse()`，items 收集到 `ctx.items`。

use dashmap::DashMap;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::stats::SpiderStats;
use super::{
    auto, control, middleware, scheduler, CrawlEvent, CrawlState, CrawlStats, Method, Request,
    Response, Spider,
};
use crate::error::Result;
use crate::fetcher::FetchMode;
use crate::http::Client;

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
    pub client: Arc<crate::fetcher::FetchClient>,
    pub fetch_mode: FetchMode,
    pub max_concurrent: usize,
    pub obey_robots: bool,
    pub engine_max_pages: usize,
    pub max_refetch_rounds: usize,
}

/// 跨 task 共享的可变状态。
pub(crate) struct EngineShared {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<Request>>>,
    pub domain_sems: Arc<DashMap<String, Arc<tokio::sync::Semaphore>>>,
    /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）
    pub proxy_clients: Arc<DashMap<String, Arc<Client>>>,
    pub control: Arc<control::EngineControl>,
    pub work_notify: Arc<tokio::sync::Notify>,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
}

/// per-run 可变状态。
pub(crate) struct EngineState {
    pub spider: Arc<dyn Spider>,
    pub stats: Arc<SpiderStats>,
    pub items: Arc<Mutex<Vec<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub start: std::time::Instant,
    pub tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    pub global_in_flight: Arc<AtomicUsize>,
}

// === 核心函数：处理单个请求 ===

/// Stage 1: 控制状态检查 + Spider 钩子（基础设施级，不可中间件化）。
async fn check_control_and_hook(ctx: &EngineContext, req: &Request) -> bool {
    // per-Engine 控制状态检查
    if ctx.shared.control.is_cancelled(&req.url).await {
        return false;
    }
    if !ctx.shared.control.wait_if_paused(&req.url).await {
        return false;
    }
    if ctx.shared.control.is_shutdown() {
        return false;
    }
    // Spider 异步钩子
    match ctx.state.spider.on_before_request(req).await {
        super::RequestAction::Proceed => true,
        super::RequestAction::Skip => false,
        super::RequestAction::Delay(d) => {
            tokio::time::sleep(d).await;
            true
        }
        super::RequestAction::Abort => {
            ctx.state.abort_flag.store(true, Ordering::SeqCst);
            false
        }
    }
}

/// 处理请求阶段：控制检查 → 中间件请求链 → 抓取。
///
/// 返回 `Some(resp)` 表示请求阶段产出响应，需由调用方交给 `process_response` 处理；
/// 返回 `None` 表示已处理完毕（Skip/Abort/错误已发送事件），无需后续。
///
/// Stages:
/// 1. 控制状态 + Spider 钩子（基础设施）
/// 2. 中间件请求链（域名/深度/robots/缓存/延迟/UA/代理 全部在此）
/// 3. 域名信号量（并发控制）+ fetch
#[tracing::instrument(skip(ctx), fields(url = %req.url))]
pub(crate) async fn process_request(ctx: &EngineContext, req: Request) -> Option<Response> {
    // 1. 控制状态 + Spider 钩子
    if !check_control_and_hook(ctx, &req).await {
        return None;
    }

    // 2. 中间件请求链（所有策略过滤在此发生）
    let mut req = req;
    let stats = &ctx.state.stats;
    if !ctx.shared.middleware_chain.is_empty() {
        let crawl_ctx = build_crawl_context(ctx);
        match ctx
            .shared
            .middleware_chain
            .run_request_middlewares(&mut req, &crawl_ctx)
            .await
        {
            middleware::MwAction::Skip => return None,
            middleware::MwAction::Abort(reason) => {
                tracing::warn!("middleware abort: {} - {}", reason, req.url);
                return None;
            }
            middleware::MwAction::Respond(cached_resp) => {
                stats.cache_hits.fetch_add(1, Ordering::SeqCst);
                record_status(stats, cached_resp.status);
                return Some(cached_resp);
            }
            middleware::MwAction::Continue
            | middleware::MwAction::Modified
            | middleware::MwAction::Refetch(_) => {}
        }
    }

    // 3. 域名信号量（并发控制）+ 抓取
    let (final_resp, last_error) = acquire_and_fetch(ctx, &req).await;

    // 4. 请求阶段收尾：失败时发送错误事件；final_resp 即返回值（与 last_error 互斥）
    if let Some(err) = last_error {
        if let Some(ref tx) = ctx.state.tx {
            let _ = tx
                .send(CrawlEvent::Error {
                    url: req.url.clone(),
                    error: err,
                })
                .await;
        }
    }
    final_resp
}

/// 处理已获取的响应：handle → Auto 升级 → items → events。
///
/// Task 3 关键改动：调用 `spider.handle(resp)`（callback 路由）而非 `spider.parse(resp)`。
/// items 同时收集到 `ctx.items`（供 `Engine::run` 返回）和 `tx`（供 `run_stream` 消费）。
#[tracing::instrument(skip(ctx, resp), fields(status = resp.status))]
pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
    let spider = &ctx.state.spider;
    let stats = &ctx.state.stats;

    if !resp.from_cache {
        stats.pages.fetch_add(1, Ordering::SeqCst);
    }
    let page_url = resp.url.clone();

    // 中间件链：响应后拦截（支持 Refetch 循环，最多 5 轮）
    let mut resp = resp;
    let mut refetch_depth = 0u32;
    if !ctx.shared.middleware_chain.is_empty() {
        loop {
            let crawl_ctx = build_crawl_context(ctx);
            match ctx
                .shared
                .middleware_chain
                .run_response_middlewares(&mut resp, &crawl_ctx)
                .await
            {
                middleware::MwAction::Skip => return,
                middleware::MwAction::Abort(reason) => {
                    tracing::warn!("response middleware abort: {} - {}", reason, page_url);
                    return;
                }
                middleware::MwAction::Refetch(new_req) => {
                    refetch_depth += 1;
                    if refetch_depth > ctx.config.max_refetch_rounds as u32 {
                        tracing::warn!(
                            "Refetch 超过 {} 轮上限，放弃: {}",
                            ctx.config.max_refetch_rounds,
                            new_req.url
                        );
                        return;
                    }
                    tracing::debug!("中间件 Refetch (round {}): {}", refetch_depth, new_req.url);
                    let (new_resp, _err) = fetch_dispatch(ctx, &new_req).await;
                    match new_resp {
                        Some(r) => {
                            resp = r;
                            continue;
                        }
                        None => return, // 获取失败，放弃
                    }
                }
                _ => break,
            }
        }
    }

    // Task 3：调用 handle()（callback 路由），而非 parse()
    let (items, follows) = spider.handle(resp).await;

    // 发送 items：经过 pipeline 链处理后收集到 ctx.items 和 tx（若有）
    for item in items {
        // 先经过 Spider 的 on_item 钩子
        let item = match spider.on_item(item).await {
            Some(i) => i,
            None => continue,
        };
        // 再经过中间件 pipeline 链
        let item = if ctx.shared.middleware_chain.is_empty() {
            Some(item)
        } else {
            let crawl_ctx = build_crawl_context(ctx);
            ctx.shared
                .middleware_chain
                .run_pipelines(item, &crawl_ctx)
                .await
        };
        if let Some(processed) = item {
            stats.items.fetch_add(1, Ordering::SeqCst);
            if let Some(ref tx) = ctx.state.tx {
                let _ = tx.send(CrawlEvent::Item(processed.clone())).await;
            }
            ctx.state.items.lock().await.push(processed);
        }
    }
    for f in follows {
        let _ = ctx.shared.follow_tx.send(f);
    }
    // 通知主循环有新工作到来
    ctx.shared.work_notify.notify_one();

    // PageScraped 事件
    if let Some(ref tx) = ctx.state.tx {
        let status_codes_snapshot = stats.status_codes_snapshot();
        let _ = tx
            .send(CrawlEvent::PageScraped {
                url: page_url,
                stats: snapshot_stats_for(stats, status_codes_snapshot, ctx.state.start),
            })
            .await;
    }
}

// === 抓取分发 ===

/// 域名信号量（并发控制）+ 单次抓取。
///
/// robots/延迟/缓存均已移至中间件，此函数仅保留不可中间件化的并发控制。
#[tracing::instrument(skip(ctx, req))]
async fn acquire_and_fetch(
    ctx: &EngineContext,
    req: &Request,
) -> (Option<Response>, Option<String>) {
    // 域名信号量（基础设施：per-domain 并发控制）
    let domain = url::Url::parse(&req.url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let sem = {
        ctx.shared
            .domain_sems
            .entry(domain)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(ctx.config.max_concurrent)))
            .clone()
    };
    let Ok(_permit) = sem.acquire_owned().await else {
        tracing::warn!("domain semaphore closed, skipping: {}", req.url);
        return (None, None);
    };

    fetch_dispatch(ctx, req).await
}

/// 抓取分发：单次 fetch，无内联重试。
///
/// 重试逻辑完全由中间件承担（即插即用）：
/// - blocked 重试：`BlockedRetryMiddleware` 通过 `MwAction::Refetch` 在 `process_response` 中处理
/// - 网络错误重试：`RetryMiddleware` 通过 `ErrorAction::Retry` 在 `process_error` 中处理
/// - 中间件链由上层配置模式（如 auto）统一注入，Engine 仅提供基础设施。
#[tracing::instrument(skip(ctx, req))]
async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>) {
    let stats = &ctx.state.stats;
    let proxy = req.proxy.clone();
    match fetch_page(
        &ctx.config.client,
        req,
        proxy.as_deref(),
        ctx.config.fetch_mode,
        &ctx.shared.rule_engine,
        &ctx.shared.proxy_clients,
    )
    .await
    {
        Ok(resp) => {
            record_status(stats, resp.status);
            if ctx.state.spider.is_blocked(&resp) {
                stats.blocked.fetch_add(1, Ordering::SeqCst);
            }
            (Some(resp), None)
        }
        Err(e) => {
            // 中间件错误处理（即插即用）
            if !ctx.shared.middleware_chain.is_empty() {
                let crawl_ctx = build_crawl_context(ctx);
                if let middleware::ErrorAction::Retry = ctx
                    .shared
                    .middleware_chain
                    .run_error_middlewares(req, &e.to_string(), &crawl_ctx)
                    .await
                {
                    // 中间件决定重试 — 引擎提供 max_retries 硬上限防止无限循环
                    let attempt = req.meta.get("_retry").and_then(|v| v.as_u64()).unwrap_or(0);
                    if attempt < ctx.state.spider.max_retries() as u64 {
                        stats.retries.fetch_add(1, Ordering::SeqCst);
                        // 重新派发：通过 follow_tx 将请求重新入队（带 _retry 计数）
                        let mut retry_req = req.clone();
                        retry_req.meta["_retry"] = serde_json::json!(attempt + 1);
                        let _ = ctx.shared.follow_tx.send(retry_req);
                        ctx.shared.work_notify.notify_one();
                        return (None, None);
                    }
                }
            }
            // 重试耗尽或无中间件
            stats.errors.fetch_add(1, Ordering::SeqCst);
            ctx.state.spider.on_error(req, &e.to_string()).await;
            (None, Some(format!("fetch failed: {} - {}", e, req.url)))
        }
    }
}

// === 辅助函数 ===

/// 从 EngineContext 构建中间件用的 CrawlContext 只读视图。
pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
    middleware::CrawlContext {
        spider_name: ctx.state.spider.name().to_string(),
        fetch_mode: ctx.config.fetch_mode,
        max_concurrent: ctx.config.max_concurrent,
        max_pages: ctx.config.engine_max_pages,
        obey_robots: ctx.config.obey_robots,
        pages_crawled: ctx.state.stats.pages.load(Ordering::SeqCst),
        errors: ctx.state.stats.errors.load(Ordering::SeqCst),
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
    start: std::time::Instant,
) -> CrawlStats {
    CrawlStats {
        items_scraped: stats.items.load(Ordering::SeqCst),
        pages_crawled: stats.pages.load(Ordering::SeqCst),
        errors: stats.errors.load(Ordering::SeqCst),
        duration: start.elapsed(),
        blocked_requests: stats.blocked.load(Ordering::SeqCst),
        retry_count: stats.retries.load(Ordering::SeqCst),
        status_code_counts: status_codes,
        offsite_requests_count: stats.offsite.load(Ordering::SeqCst),
        cache_hits: stats.cache_hits.load(Ordering::SeqCst),
        ..Default::default()
    }
}

/// Checkpoint 保存。
pub(crate) async fn save_checkpoint(
    store: &crate::storage::Store,
    spider_name: &str,
    sched: &scheduler::Scheduler,
    stats: &Arc<SpiderStats>,
) {
    let pending = sched.pending_urls().await;
    let seen = sched.seen_urls().await; // 持久化 seen 去重集合
    let snapshot = snapshot_stats_for(stats, HashMap::new(), stats.start);
    // 手动构造 CrawlState 填入 seen_urls；
    // `CrawlState::from_stats` 硬编码 seen_urls 为空，不能直接用。
    let state = CrawlState {
        spider_name: spider_name.to_string(),
        pending_urls: pending,
        seen_urls: seen,
        items_scraped: snapshot.items_scraped,
        pages_crawled: snapshot.pages_crawled,
        errors: snapshot.errors,
        duration_ms: snapshot.duration.as_millis(),
        saved_at: chrono::Utc::now(),
    };
    match bincode::serialize(&state) {
        Ok(blob) => {
            if let Err(e) = store.save_checkpoint(spider_name, &blob, state.saved_at.timestamp()) {
                tracing::warn!("checkpoint 保存失败: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("checkpoint 序列化失败: {}", e);
        }
    }
}

// === fetch_page（模式分发）===

#[doc(hidden)]
pub async fn fetch_page(
    fetch_client: &crate::fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    mode: FetchMode,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
    proxy_clients: &DashMap<String, Arc<Client>>,
) -> Result<Response> {
    // 1. 中间件设置的模式覆盖优先（如 StealthUpgradeMiddleware Refetch 时设置）
    if let Some(override_mode) = req.fetch_mode_override {
        return fetch_page_inner(fetch_client, req, proxy_url, override_mode, proxy_clients).await;
    }

    // 2. Auto 模式：rule_engine 缓存 → HTTP 先行，blocked 检测由 StealthUpgradeMiddleware 承担
    if mode == FetchMode::Auto {
        let resolved = { rule_engine.lock().await.resolve(&req.url) };
        if let Some(cached_mode) = resolved {
            return fetch_page_inner(fetch_client, req, proxy_url, cached_mode, proxy_clients)
                .await;
        }
        // HTTP 先行（升级由 DynamicUpgradeMiddleware 中间件通过 Refetch 触发）
        let resp =
            fetch_page_inner(fetch_client, req, proxy_url, FetchMode::Http, proxy_clients).await?;
        return Ok(resp);
    }

    // 3. 非 Auto：直接按指定模式抓取
    fetch_page_inner(fetch_client, req, proxy_url, mode, proxy_clients).await
}

/// 内部实际抓取（根据模式分发）。
#[doc(hidden)]
pub async fn fetch_page_inner(
    fetch_client: &crate::fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    mode: FetchMode,
    proxy_clients: &DashMap<String, Arc<Client>>,
) -> Result<Response> {
    // 浏览器模式：通过 BrowserPool 复用实例（RAII 自动归还，无泄漏）
    if mode == FetchMode::Dynamic || mode == FetchMode::Stealth {
        let solve_cf = mode == FetchMode::Stealth;
        let fetch_method = match req.method {
            crate::crawl::Method::Get => crate::fetcher::Method::Get,
            crate::crawl::Method::Post => crate::fetcher::Method::Post,
            crate::crawl::Method::Put => crate::fetcher::Method::Put,
            crate::crawl::Method::Delete => crate::fetcher::Method::Delete,
        };
        let fetch_req = crate::fetcher::Request {
            url: req.url.clone(),
            method: fetch_method,
            headers: req.headers.clone(),
            body: req.body.clone(),
            ..Default::default()
        };
        let resp = fetch_client.fetch_browser(&fetch_req, solve_cf).await?;
        return Ok(Response {
            url: resp.url.clone(),
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body.clone(),
            title: resp.title.clone(),
            cookies: resp.cookies.clone(),
            request: req.clone(),
            content_type: String::new(),
            from_cache: false,
        });
    }

    // Http 模式
    let base_client = fetch_client.http();

    // 代理 Client 缓存：相同 proxy URL 复用已建立的连接，避免每请求 TLS 握手。
    // 用 Entry match 保证原子性：Vacant 分支在持 shard 锁期间 build + insert，
    // 消除 get→entry 之间另一 task 抢先插入导致的偶发多余 Client 构建。
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        use dashmap::mapref::entry::Entry;
        match proxy_clients.entry(proxy.to_string()) {
            Entry::Occupied(o) => Some(o.get().clone()),
            Entry::Vacant(v) => {
                let new_client = Client::builder()
                    .timeout(base_client.config_ref().timeout)
                    .proxy(proxy)
                    .build()?;
                Some(v.insert(Arc::new(new_client)).clone())
            }
        }
    } else {
        None
    };
    let use_client: &Client = match &proxy_client {
        Some(c) => c.as_ref(),
        None => base_client,
    };

    // 收集中间件/请求级 headers（如 UaRotationMiddleware 设置的 User-Agent，
    // 或 CookieChallengeMiddleware 累积的 Cookie）
    let extra_headers: Vec<(String, String)> = req
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let resp = match req.method {
        Method::Get => use_client.get(&req.url, &extra_headers).await?,
        Method::Post => {
            use_client
                .post(&req.url, req.body.as_deref(), None, &extra_headers)
                .await?
        }
        Method::Put => {
            use_client
                .put(&req.url, req.body.as_deref(), None, &extra_headers)
                .await?
        }
        Method::Delete => use_client.delete(&req.url, &extra_headers).await?,
    };

    Ok(Response {
        url: resp.url.clone(),
        status: resp.status,
        headers: resp.headers.clone(),
        body: resp.body.clone(),
        title: None,
        cookies: Vec::new(),
        request: req.clone(),
        content_type: resp
            .headers
            .get("content-type")
            .cloned()
            .unwrap_or_default(),
        from_cache: false,
    })
}

// === InFlightGuard ===

pub(crate) struct InFlightGuard {
    pub counter: Arc<AtomicUsize>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::time::Instant;

    /// 最小 Spider：parse 返回空，不产出 items/follows，避免触碰事件通道。
    struct DummySpider;

    #[async_trait]
    impl Spider for DummySpider {
        fn name(&self) -> &str {
            "dummy"
        }
        fn start_urls(&self) -> Vec<String> {
            vec![]
        }
        async fn parse(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
            (vec![], vec![])
        }
    }

    /// 构造最小 EngineContext（单 Spider，Http 模式，无事件通道）。
    /// 返回上下文与对应 stats 的 Arc 克隆，便于测试断言计数器。
    fn make_ctx() -> (EngineContext, Arc<SpiderStats>) {
        let stats = Arc::new(SpiderStats::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let ctx = EngineContext {
            config: EngineConfig {
                client: Arc::new(
                    crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                        .expect("build fetch client"),
                ),
                fetch_mode: FetchMode::Http,
                max_concurrent: 8,
                obey_robots: false,
                engine_max_pages: 100,
                max_refetch_rounds: 5,
            },
            shared: EngineShared {
                sched: Arc::new(scheduler::Scheduler::new()),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                domain_sems: Arc::new(DashMap::new()),
                proxy_clients: Arc::new(dashmap::DashMap::new()),
                control: Arc::new(control::EngineControl::new()),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
                rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
            },
            state: EngineState {
                spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                stats: stats.clone(),
                items: Arc::new(Mutex::new(Vec::new())),
                abort_flag: Arc::new(AtomicBool::new(false)),
                start: Instant::now(),
                tx: None,
                global_in_flight: Arc::new(AtomicUsize::new(0)),
            },
        };
        (ctx, stats)
    }

    /// 构造最小 Response，仅 from_cache 字段可变。
    fn make_resp(from_cache: bool) -> Response {
        Response {
            url: "http://example.com/page".into(),
            status: 200,
            headers: HashMap::new(),
            body: vec![],
            title: None,
            cookies: Vec::new(),
            request: Request::get("http://example.com/page"),
            content_type: String::new(),
            from_cache,
        }
    }

    /// 缓存命中（from_cache=true）时 stats.pages 不应递增。
    #[tokio::test]
    async fn process_response_from_cache_does_not_increment_pages() {
        let (ctx, stats) = make_ctx();
        let resp = make_resp(true);
        process_response(&ctx, resp).await;
        assert_eq!(
            stats.pages.load(Ordering::SeqCst),
            0,
            "缓存命中时 pages 不应递增"
        );
    }

    /// 非缓存响应（from_cache=false）时 stats.pages 应递增。
    #[tokio::test]
    async fn process_response_not_from_cache_increments_pages() {
        let (ctx, stats) = make_ctx();
        let resp = make_resp(false);
        process_response(&ctx, resp).await;
        assert_eq!(
            stats.pages.load(Ordering::SeqCst),
            1,
            "非缓存响应 pages 应递增到 1"
        );
    }

    /// Task 3：验证 save_checkpoint 把 Scheduler 的 seen_urls 集合写入持久化 blob。
    ///
    /// RED：当前 save_checkpoint 用 `CrawlState::from_stats`，其 seen_urls 硬编码为空，
    /// 故反序列化后的 state.seen_urls 必为空，断言失败。
    #[tokio::test]
    async fn save_checkpoint_persists_seen_urls() {
        let store = crate::storage::Store::open_in_memory().expect("open in-memory store");
        let sched = scheduler::Scheduler::new();
        // push 两个 URL：进入 heap 与 seen 集合
        sched.push(Request::get("https://example.com/a")).await;
        sched.push(Request::get("https://example.com/b")).await;

        let stats = Arc::new(SpiderStats::new());
        save_checkpoint(&store, "seen_persist_spider", &sched, &stats).await;

        let blob = store
            .load_checkpoint("seen_persist_spider")
            .expect("load checkpoint ok")
            .expect("checkpoint should exist");
        let state: CrawlState = bincode::deserialize(&blob).expect("deserialize state");
        assert!(
            state.seen_urls.contains("https://example.com/a"),
            "seen_urls 必须包含已爬 URL a，当前 seen = {:?}",
            state.seen_urls
        );
        assert!(
            state.seen_urls.contains("https://example.com/b"),
            "seen_urls 必须包含已爬 URL b，当前 seen = {:?}",
            state.seen_urls
        );
    }
}
