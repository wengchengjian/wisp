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

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use dashmap::DashMap;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::error::Result;
use crate::http::{self, Client};
use crate::fetcher::FetchMode;
use super::{
    Spider, SpiderRequest, SpiderResponse, Method,
    CrawlStats, CrawlEvent, CrawlState,
    auto, scheduler, robots, control, middleware,
};
use super::stats::SpiderStats;

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
    pub client: Arc<Client>,
    pub fetcher_config: http::Config,
    pub fetch_mode: FetchMode,
    pub max_concurrent: usize,
    pub max_depth: u32,
    pub obey_robots: bool,
    pub engine_max_pages: usize,
    pub max_refetch_rounds: usize,
    pub dev_mode: bool,
    pub allowed: Arc<HashSet<String>>,
    pub auto_excludes: HashSet<String>,
}

/// 跨 task 共享的可变状态。
pub(crate) struct EngineShared {
    pub sched: Arc<scheduler::Scheduler>,
    pub robots_cache: Arc<Mutex<robots::RobotsCache>>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<SpiderRequest>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<SpiderRequest>>>,
    pub domain_sems: Arc<DashMap<String, Arc<tokio::sync::Semaphore>>>,
    /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）
    pub proxy_clients: Arc<dashmap::DashMap<String, Arc<Client>>>,
    pub cache_store: Option<Arc<crate::storage::Store>>,
    pub request_cache: Option<super::request_cache::RequestCache>,
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

/// 请求过滤结果。
enum FilterAction {
    /// 继续处理
    Proceed,
    /// 跳过此请求
    Skip,
    /// 中止整个爬取
    Abort,
    /// 延迟后继续
    Delay(Duration),
}

/// 缓存检查结果。
enum CacheResult {
    /// 缓存命中，直接处理此响应
    Hit(Box<SpiderResponse>),
    /// 未命中，继续网络请求
    Miss,
}

/// Stage 1: 请求过滤检查（域名/深度/控制状态/异步钩子）。
///
/// 返回 FilterAction::Proceed 表示继续；其他表示终止此请求的处理。
async fn check_request_filters(ctx: &EngineContext, req: &SpiderRequest) -> FilterAction {
    let stats = &ctx.state.stats;
    let allowed = &ctx.config.allowed;

    // 1. 域名过滤
    if !allowed.is_empty() {
        if let Ok(parsed) = url::Url::parse(&req.url) {
            if let Some(host) = parsed.host_str() {
                if !allowed.contains(host) {
                    stats.offsite.fetch_add(1, Ordering::SeqCst);
                    return FilterAction::Skip;
                }
            }
        }
    }

    // 1.5. 深度检查
    if req.depth > ctx.config.max_depth {
        return FilterAction::Skip;
    }

    // 1.6. per-Engine 控制状态检查
    if ctx.shared.control.is_cancelled(&req.url).await { return FilterAction::Skip; }
    if !ctx.shared.control.wait_if_paused(&req.url).await { return FilterAction::Skip; }
    if ctx.shared.control.is_shutdown() { return FilterAction::Skip; }

    // 1.7. 异步钩子检查
    match ctx.state.spider.on_before_request(req).await {
        super::RequestAction::Proceed => FilterAction::Proceed,
        super::RequestAction::Skip => FilterAction::Skip,
        super::RequestAction::Delay(d) => FilterAction::Delay(d),
        super::RequestAction::Abort => FilterAction::Abort,
    }
}

/// Stage 3: 缓存检查（RequestCache 内存缓存 + dev_mode SQLite 缓存）。
///
/// 返回 CacheResult::Hit(resp) 表示命中缓存，直接处理响应；
/// 返回 CacheResult::Miss 表示未命中，继续网络请求。
async fn check_request_caches(ctx: &EngineContext, req: &SpiderRequest, method_str: &str) -> CacheResult {
    let stats = &ctx.state.stats;

    // 内存缓存检查 (RequestCache)
    if let Some(ref rc) = ctx.shared.request_cache {
        if let Some(entry) = rc.get(method_str, &req.url).await {
            let resp = SpiderResponse {
                url: req.url.clone(),
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
                request: req.clone(),
                tracker: None,
                from_cache: true,
            };
            stats.cache_hits.fetch_add(1, Ordering::SeqCst);
            record_status(stats, resp.status);
            return CacheResult::Hit(Box::new(resp));
        }
    }

    // 开发模式 SQLite 缓存检查
    if ctx.config.dev_mode {
        if let Some(ref store) = ctx.shared.cache_store {
            if let Some(cached) = store.load_cached_response(&req.url, method_str).ok().flatten() {
                let resp = SpiderResponse {
                    url: req.url.clone(),
                    status: cached.status,
                    headers: cached.headers,
                    body: cached.body,
                    request: req.clone(),
                    tracker: None,
                    from_cache: true,
                };
                stats.cache_hits.fetch_add(1, Ordering::SeqCst);
                record_status(stats, resp.status);
                return CacheResult::Hit(Box::new(resp));
            }
        }
    }

    CacheResult::Miss
}

/// Stage 4: robots 检查 → 域名信号量 → 延迟 → fetch_dispatch → 缓存写入。
///
/// 返回 (Option<SpiderResponse>, Option<String>) — 成功返回 resp，失败返回 error。
async fn acquire_and_fetch(
    ctx: &EngineContext,
    req: &SpiderRequest,
    method_str: &str,
) -> (Option<SpiderResponse>, Option<String>) {
    let obey_robots = ctx.config.obey_robots;
    let max_concurrent = ctx.config.max_concurrent;

    // robots 检查
    if obey_robots {
        let allowed_flag = {
            let mut rc = ctx.shared.robots_cache.lock().await;
            rc.is_allowed(&ctx.config.client, &req.url).await
        };
        if !allowed_flag {
            return (None, None);
        }
    }

    // 域名信号量
    let domain = url::Url::parse(&req.url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let sem = {
        ctx.shared.domain_sems
            .entry(domain)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
            .clone()
    };
    let Ok(_permit) = sem.acquire_owned().await else {
        tracing::warn!("domain semaphore closed, skipping: {}", req.url);
        return (None, None);
    };

    // 延迟
    apply_delay(ctx, &req.url, &ctx.state.spider, obey_robots).await;

    // 带重试的抓取
    let (resp, err) = fetch_dispatch(ctx, req).await;

    // 开发模式缓存保存
    if ctx.config.dev_mode {
        if let Some(ref store) = ctx.shared.cache_store {
            if let Some(ref resp) = resp {
                let cached = crate::storage::CachedResponse {
                    status: resp.status,
                    headers: resp.headers.clone(),
                    body: resp.body.clone(),
                    cached_at: chrono::Utc::now().timestamp(),
                };
                let _ = store.save_cached_response(&req.url, method_str, &cached);
            }
        }
    }

    // 写入 RequestCache
    if let Some(ref rc) = ctx.shared.request_cache {
        if let Some(ref resp) = resp {
            rc.put(method_str, &req.url, super::request_cache::CachedEntry {
                status: resp.status,
                headers: resp.headers.clone(),
                body: resp.body.clone(),
            }).await;
        }
    }

    (resp, err)
}

/// 处理单个请求的完整流程（编排层）。
///
/// Stages:
/// 1. check_request_filters — 域名/深度/控制/钩子
/// 2. 中间件请求拦截（可能短路）
/// 3. check_request_caches — RequestCache + dev_mode 缓存
/// 4. acquire_and_fetch — robots + 信号量 + 延迟 + fetch_dispatch + 缓存写入
/// 5. process_response — handle + items + events（已存在）
pub(crate) async fn process_request(ctx: &EngineContext, req: SpiderRequest) {
    // 1. 过滤检查
    match check_request_filters(ctx, &req).await {
        FilterAction::Proceed => {}
        FilterAction::Skip => return,
        FilterAction::Abort => {
            ctx.state.abort_flag.store(true, Ordering::SeqCst);
            return;
        }
        FilterAction::Delay(d) => { tokio::time::sleep(d).await; }
    }

    // 2. 中间件请求拦截
    let mut req = req;
    let stats = &ctx.state.stats;
    if !ctx.shared.middleware_chain.is_empty() {
        let crawl_ctx = build_crawl_context(ctx);
        match ctx.shared.middleware_chain.run_request_middlewares(&mut req, &crawl_ctx).await {
            middleware::MwAction::Skip => return,
            middleware::MwAction::Abort(reason) => {
                tracing::warn!("middleware abort: {} - {}", reason, req.url);
                return;
            }
            middleware::MwAction::Respond(cached_resp) => {
                // 中间件短路（如缓存命中），跳过网络请求直接处理响应
                stats.cache_hits.fetch_add(1, Ordering::SeqCst);
                record_status(stats, cached_resp.status);
                return process_response(ctx, cached_resp, &req).await;
            }
            _ => {}
        }
    }

    // 提前计算 method_str（缓存查询与写入都需要）
    let method_str = req.method.as_str();

    // 3. 缓存检查
    match check_request_caches(ctx, &req, method_str).await {
        CacheResult::Hit(resp) => {
            return process_response(ctx, *resp, &req).await;
        }
        CacheResult::Miss => {}
    }

    // 4. robots + 信号量 + 延迟 + 抓取 + 缓存写入
    let (final_resp, last_error) = acquire_and_fetch(ctx, &req, method_str).await;

    // 5. 处理结果
    if let Some(resp) = final_resp {
        process_response(ctx, resp, &req).await;
    } else if let Some(err) = last_error {
        if let Some(ref tx) = ctx.state.tx {
            let _ = tx.send(CrawlEvent::Error { url: req.url.clone(), error: err }).await;
        }
    }
}

/// 处理已获取的响应：handle → Auto 升级 → items → events。
///
/// Task 3 关键改动：调用 `spider.handle(resp)`（callback 路由）而非 `spider.parse(resp)`。
/// items 同时收集到 `ctx.items`（供 `Engine::run` 返回）和 `tx`（供 `run_stream` 消费）。
pub(crate) async fn process_response(ctx: &EngineContext, resp: SpiderResponse, req: &SpiderRequest) {
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
            match ctx.shared.middleware_chain.run_response_middlewares(&mut resp, &crawl_ctx).await {
                middleware::MwAction::Skip => return,
                middleware::MwAction::Abort(reason) => {
                    tracing::warn!("response middleware abort: {} - {}", reason, page_url);
                    return;
                }
                middleware::MwAction::Refetch(new_req) => {
                    refetch_depth += 1;
                    if refetch_depth > ctx.config.max_refetch_rounds as u32 {
                        tracing::warn!("Refetch 超过 {} 轮上限，放弃: {}", ctx.config.max_refetch_rounds, new_req.url);
                        return;
                    }
                    tracing::debug!("中间件 Refetch (round {}): {}", refetch_depth, new_req.url);
                    let (new_resp, _err) = fetch_dispatch(ctx, &new_req).await;
                    match new_resp {
                        Some(r) => { resp = r; continue; }
                        None => return, // 获取失败，放弃
                    }
                }
                _ => break,
            }
        }
    }

    let tracker_ref = resp.tracker.clone();
    // Task 3：调用 handle()（callback 路由），而非 parse()
    let (mut items, mut follows) = spider.handle(resp).await;

    // Auto 升级检查
    if ctx.config.fetch_mode == FetchMode::Auto {
        if let Some(result) = auto_upgrade_check(ctx, &tracker_ref, &page_url, req).await {
            items = result.0;
            follows = result.1;
        }
    }

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
            ctx.shared.middleware_chain.run_pipelines(item, &crawl_ctx).await
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
        let _ = tx.send(CrawlEvent::PageScraped {
            url: page_url,
            stats: snapshot_stats_for(stats, status_codes_snapshot, ctx.state.start),
        }).await;
    }
}

// === 抓取分发 ===

/// 抓取分发：fetch → blocked 检测 → transport 级重试 fallback。
///
/// 注意：blocked 重试和 error 重试已分别由 BlockedRetryMiddleware / RetryMiddleware
/// 通过 Refetch / ErrorAction::Retry 承担。此函数保留为无中间件时的 fallback。
async fn fetch_dispatch(ctx: &EngineContext, req: &SpiderRequest) -> (Option<SpiderResponse>, Option<String>) {
    let spider = &ctx.state.spider;
    let stats = &ctx.state.stats;
    let fetch_mode = ctx.config.fetch_mode;
    let fetcher_config = &ctx.config.fetcher_config;
    let rule_engine = &ctx.shared.rule_engine;
    let max_retries = spider.max_retries();
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        let proxy = req.proxy.clone();
        match fetch_page(&ctx.config.client, req, proxy.as_deref(), fetch_mode, fetcher_config, rule_engine, &ctx.shared.proxy_clients).await {
            Ok(resp) => {
                record_status(stats, resp.status);
                if spider.is_blocked(&resp) {
                    stats.blocked.fetch_add(1, Ordering::SeqCst);
                    // attempt 从 1 起；attempt <= max_retries 表示还能重试，
                    // 故 max_retries=3 时实际尝试 4 次（attempt 1..=4）。
                    if attempt <= max_retries {
                        stats.retries.fetch_add(1, Ordering::SeqCst);
                        let delay = spider.download_delay();
                        if delay > Duration::ZERO { tokio::time::sleep(delay).await; }
                        tracing::warn!("blocked (status={}, attempt={}/{}), retrying: {}", resp.status, attempt, max_retries, req.url);
                        continue;
                    }
                    stats.errors.fetch_add(1, Ordering::SeqCst);
                    spider.on_error(req, &format!("blocked after {} retries (status={})", max_retries, resp.status)).await;
                    return (None, Some(format!(
                        "blocked after {} retries (status={}, total attempts={})",
                        max_retries, resp.status, attempt
                    )));
                }
                return (Some(resp), None);
            }
            Err(e) => {
                // 中间件链：错误处理（可决定重试或放弃）
                if !ctx.shared.middleware_chain.is_empty() {
                    let crawl_ctx = build_crawl_context(ctx);
                    if let middleware::ErrorAction::Retry = ctx.shared.middleware_chain.run_error_middlewares(req, &e.to_string(), &crawl_ctx).await {
                        stats.retries.fetch_add(1, Ordering::SeqCst);
                        continue;
                    }
                }
                if attempt <= max_retries {
                    stats.retries.fetch_add(1, Ordering::SeqCst);
                    let delay = spider.download_delay();
                    if delay > Duration::ZERO { tokio::time::sleep(delay).await; }
                    tracing::warn!("fetch error (attempt={}/{}): {} - {}", attempt, max_retries, e, req.url);
                    continue;
                }
                stats.errors.fetch_add(1, Ordering::SeqCst);
                spider.on_error(req, &e.to_string()).await;
                return (None, Some(format!(
                    "fetch failed after {} retries (total attempts={}): {}",
                    max_retries, attempt, e
                )));
            }
        }
    }
}

// === Auto 升级检查 ===

/// Auto 模式：检查 tracker 中选择器是否有 0 匹配，若有则升级 Dynamic 重取。
/// 返回 Some((items, follows)) 表示已升级并重新 parse；None 表示无需升级。
async fn auto_upgrade_check(
    ctx: &EngineContext,
    tracker: &Option<Arc<std::sync::Mutex<auto::SelectorTracker>>>,
    page_url: &str,
    req: &SpiderRequest,
) -> Option<(Vec<Value>, Vec<SpiderRequest>)> {
    let spider = &ctx.state.spider;
    let fetcher_config = &ctx.config.fetcher_config;
    let rule_engine = &ctx.shared.rule_engine;
    let auto_exclude = &ctx.config.auto_excludes;
    let tracker = tracker.as_ref()?;
    let needs = tracker.lock().unwrap_or_else(|e| e.into_inner()).needs_upgrade(auto_exclude);

    if needs {
        {
            let mut engine = rule_engine.lock().await;
            engine.learn(page_url, FetchMode::Dynamic);
        }
        tracing::info!("Auto: '{}' 选择器无内容，升级 Dynamic", page_url);
        let proxy = req.proxy.clone();
        let dynamic_resp = fetch_page_inner(
            &ctx.config.client, req, proxy.as_deref(), FetchMode::Dynamic, fetcher_config, &ctx.shared.proxy_clients,
        ).await;
        if let Ok(new_resp) = dynamic_resp {
            // Auto 升级时也走 handle() 路由（保持一致）
            let (items, follows) = spider.handle(new_resp).await;
            return Some((items, follows));
        }
        None
    } else {
        let mut engine = rule_engine.lock().await;
        engine.learn(page_url, FetchMode::Http);
        None
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
pub fn record_status(stats: &Arc<SpiderStats>, status: u16) {
    stats
        .status_codes
        .entry(status)
        .and_modify(|c| { c.fetch_add(1, Ordering::Relaxed); })
        .or_insert(AtomicUsize::new(1));
}

async fn apply_delay(ctx: &EngineContext, url: &str, spider: &Arc<dyn Spider>, obey_robots: bool) {
    let mut delay = spider.download_delay();
    if obey_robots {
        let robots_delay = {
            let mut rc = ctx.shared.robots_cache.lock().await;
            rc.crawl_delay(&ctx.config.client, url).await
        };
        if let Some(secs) = robots_delay {
            let robots_dur = Duration::from_secs_f64(secs);
            if robots_dur > delay { delay = robots_dur; }
        }
    }
    if delay > Duration::ZERO {
        tokio::time::sleep(delay).await;
    }
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

pub async fn fetch_page(
    client: &Client,
    req: &SpiderRequest,
    proxy_url: Option<&str>,
    mode: FetchMode,
    config: &http::Config,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
    proxy_clients: &dashmap::DashMap<String, Arc<Client>>,
) -> Result<SpiderResponse> {
    // 1. 中间件设置的模式覆盖优先（如 StealthUpgradeMiddleware Refetch 时设置）
    if let Some(override_mode) = req.fetch_mode_override {
        return fetch_page_inner(client, req, proxy_url, override_mode, config, proxy_clients).await;
    }

    // 2. Auto 模式：rule_engine 缓存 → HTTP 先行，blocked 检测由 StealthUpgradeMiddleware 承担
    if mode == FetchMode::Auto {
        let resolved = { rule_engine.lock().await.resolve(&req.url) };
        if let Some(cached_mode) = resolved {
            return fetch_page_inner(client, req, proxy_url, cached_mode, config, proxy_clients).await;
        }
        // HTTP 先行，附加 SelectorTracker（用于 auto_upgrade_check 检测选择器匹配）
        let resp = fetch_page_inner(client, req, proxy_url, FetchMode::Http, config, proxy_clients).await?;
        let tracker = Arc::new(std::sync::Mutex::new(auto::SelectorTracker::new()));
        return Ok(SpiderResponse { tracker: Some(tracker), ..resp });
    }

    // 3. 非 Auto：直接按指定模式抓取
    fetch_page_inner(client, req, proxy_url, mode, config, proxy_clients).await
}

/// 内部实际抓取（根据模式分发）。
pub async fn fetch_page_inner(
    client: &Client,
    req: &SpiderRequest,
    proxy_url: Option<&str>,
    mode: FetchMode,
    config: &http::Config,
    proxy_clients: &dashmap::DashMap<String, Arc<Client>>,
) -> Result<SpiderResponse> {
    if mode == FetchMode::Dynamic || mode == FetchMode::Stealth {
        let mut builder = match mode {
            FetchMode::Dynamic => crate::fetcher::Fetcher::dynamic(),
            FetchMode::Stealth => crate::fetcher::Fetcher::stealth(),
            _ => unreachable!(),
        };
        builder = builder.timeout(config.timeout);
        if let Some(proxy) = proxy_url {
            builder = builder.proxy(proxy);
        }
        let resp = builder.get(&req.url).await?;
        return Ok(SpiderResponse {
            url: resp.url.clone(),
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body.clone(),
            request: req.clone(),
            tracker: None,
            from_cache: false,
        });
    }

    // Http 模式
    // 代理 Client 缓存：相同 proxy URL 复用已建立的连接，避免每请求 TLS 握手
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        if let Some(c) = proxy_clients.get(proxy) {
            Some(c.clone())
        } else {
            // 慢路径：构建新 client（可能失败，错误向上传播）
            let new_client = Client::builder()
                .timeout(client.config_ref().timeout)
                .proxy(proxy)
                .build()?;
            let arc = Arc::new(new_client);
            // 并发安全：若另一 task 已插入，用已存在的；否则用新建的
            Some(proxy_clients.entry(proxy.to_string()).or_insert(arc).clone())
        }
    } else {
        None
    };
    let use_client: &Client = match &proxy_client {
        Some(c) => c.as_ref(),
        None => client,
    };

    // 收集中间件/请求级 headers（如 UaRotationMiddleware 设置的 User-Agent，
    // 或 CookieChallengeMiddleware 累积的 Cookie）
    let extra_headers: Vec<(String, String)> = req.headers.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let resp = if extra_headers.is_empty() {
        match req.method {
            Method::Get => use_client.get(&req.url).await?,
            Method::Post => use_client.post(&req.url, req.body.as_deref(), None).await?,
            Method::Put => use_client.put(&req.url, req.body.as_deref(), None).await?,
            Method::Delete => use_client.delete(&req.url).await?,
        }
    } else {
        match req.method {
            Method::Get => use_client.get_with_headers(&req.url, &extra_headers).await?,
            Method::Post => use_client.post_with_headers(&req.url, req.body.as_deref(), None, &extra_headers).await?,
            Method::Put => use_client.put(&req.url, req.body.as_deref(), None).await?,
            Method::Delete => use_client.delete(&req.url).await?,
        }
    };

    Ok(SpiderResponse {
        url: resp.url.clone(),
        status: resp.status,
        headers: resp.headers.clone(),
        body: resp.body.clone(),
        request: req.clone(),
        tracker: None,
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
        fn name(&self) -> &str { "dummy" }
        fn start_urls(&self) -> Vec<String> { vec![] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            (vec![], vec![])
        }
    }

    /// 构造最小 EngineContext（单 Spider，Http 模式，无事件通道）。
    /// 返回上下文与对应 stats 的 Arc 克隆，便于测试断言计数器。
    fn make_ctx() -> (EngineContext, Arc<SpiderStats>) {
        let stats = Arc::new(SpiderStats::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();
        let ctx = EngineContext {
            config: EngineConfig {
                client: Arc::new(Client::new().expect("build http client")),
                fetcher_config: http::Config::default(),
                fetch_mode: FetchMode::Http,
                max_concurrent: 8,
                max_depth: u32::MAX,
                obey_robots: false,
                engine_max_pages: 100,
                max_refetch_rounds: 5,
                dev_mode: false,
                allowed: Arc::new(HashSet::new()),
                auto_excludes: HashSet::new(),
            },
            shared: EngineShared {
                sched: Arc::new(scheduler::Scheduler::new()),
                robots_cache: Arc::new(Mutex::new(robots::RobotsCache::new())),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                domain_sems: Arc::new(DashMap::new()),
                proxy_clients: Arc::new(dashmap::DashMap::new()),
                cache_store: None,
                request_cache: None,
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

    /// 构造最小 SpiderResponse，仅 from_cache 字段可变。
    fn make_resp(from_cache: bool) -> SpiderResponse {
        SpiderResponse {
            url: "http://example.com/page".into(),
            status: 200,
            headers: HashMap::new(),
            body: vec![],
            request: SpiderRequest::get("http://example.com/page"),
            tracker: None,
            from_cache,
        }
    }

    /// 缓存命中（from_cache=true）时 stats.pages 不应递增。
    #[tokio::test]
    async fn process_response_from_cache_does_not_increment_pages() {
        let (ctx, stats) = make_ctx();
        let req = SpiderRequest::get("http://example.com/page");
        let resp = make_resp(true);
        process_response(&ctx, resp, &req).await;
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
        let req = SpiderRequest::get("http://example.com/page");
        let resp = make_resp(false);
        process_response(&ctx, resp, &req).await;
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
        sched.push(SpiderRequest::get("https://example.com/a")).await;
        sched.push(SpiderRequest::get("https://example.com/b")).await;

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
