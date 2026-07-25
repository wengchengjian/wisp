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

// 注：per-domain 信号量已删除。全局并发由 buffer_unordered(buffer_ceiling) 控制，
// 动态调整由 autoscale 负责。多域名公平性由用户通过 Request::priority 或 download_delay 管理。
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::stats::SpiderStats;
use super::{
    auto, control, middleware, scheduler, CrawlEvent, CrawlState, CrawlStats, Request, Response,
    Spider,
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
    /// 网络错误重试上限（fetch 失败后同步重试，由 engine 维护 retry_count）。
    ///
    /// 与 `max_refetch_rounds`（响应中间件 Refetch 上限）独立：
    /// - `max_retries`：fetch_page 失败后，engine 在 fetch_dispatch 内同步重试的次数上限
    /// - `max_refetch_rounds`：响应成功后，process_response 内 Refetch 循环的次数上限
    pub max_retries: u32,
}

/// 跨 task 共享的可变状态。
pub(crate) struct EngineShared {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<Request>>>,
    /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）。
    ///
    /// ND-009-SEC：使用 moka::sync::Cache 限制最大条目数，防止攻击者注入大量
    /// 不同 proxy URL 导致无界增长 OOM。默认上限 1024，超限时 LRU 淘汰。
    pub proxy_clients: Arc<moka::sync::Cache<String, Arc<Client>>>,
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
#[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
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
                tracing::warn!("middleware abort: {} - {}", reason, sanitize_url(&req.url));
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

    // 3. 抓取（全局并发由 buffer_unordered 控制，无需 per-domain 信号量）
    let (final_resp, last_error) = fetch_dispatch(ctx, &req).await;

    // 4. 请求阶段收尾：失败时发送错误事件；final_resp 即返回值（与 last_error 互斥）
    if let Some(err) = last_error {
        if let Some(ref tx) = ctx.state.tx {
            let _ = tx
                .send(CrawlEvent::Error {
                    url: sanitize_url(&req.url), // ND-007-SEC：事件中脱敏 URL 凭据
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
#[tracing::instrument(level = "trace", skip(ctx, resp), fields(status = resp.status))]
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
                    tracing::warn!(
                        "response middleware abort: {} - {}",
                        reason,
                        sanitize_url(&page_url)
                    );
                    return;
                }
                middleware::MwAction::Refetch(new_req) => {
                    refetch_depth += 1;
                    if refetch_depth > ctx.config.max_refetch_rounds as u32 {
                        tracing::warn!(
                            "Refetch 超过 {} 轮上限，放弃: {}",
                            ctx.config.max_refetch_rounds,
                            sanitize_url(&new_req.url)
                        );
                        // ND-007-CORR：发送 Error 事件，不静默丢弃
                        emit_error_event(
                            ctx,
                            &new_req.url,
                            &format!(
                                "refetch exceeded {} rounds limit",
                                ctx.config.max_refetch_rounds
                            ),
                        );
                        return;
                    }
                    tracing::debug!(
                        "中间件 Refetch (round {}): {}",
                        refetch_depth,
                        sanitize_url(&new_req.url)
                    );
                    // ND-007-CORR：保留错误上下文，不再用 _err 丢弃
                    let (new_resp, err) = fetch_dispatch(ctx, &new_req).await;
                    match new_resp {
                        Some(r) => {
                            resp = r;
                            continue;
                        }
                        None => {
                            // ND-007-CORR：refetch 失败时发送 Error 事件，不静默丢弃
                            let err_msg =
                                err.unwrap_or_else(|| "refetch failed (unknown error)".to_string());
                            tracing::warn!(
                                "Refetch 失败，放弃: {} - {}",
                                sanitize_url(&new_req.url),
                                err_msg
                            );
                            emit_error_event(ctx, &new_req.url, &err_msg);
                            return;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    // Task 3：调用 handle()（callback 路由），而非 parse()
    let (items, follows) = spider.handle(resp).await;

    // 发送 items：经过 pipeline 链处理后收集到 ctx.items 和 tx（若有）
    // ND-009-PERF：crawl_ctx 在循环外构建一次，避免每 item 重建
    let pipeline_crawl_ctx = if ctx.shared.middleware_chain.is_empty() {
        None
    } else {
        Some(build_crawl_context(ctx))
    };
    for item in items {
        // 先经过 Spider 的 on_item 钩子
        let item = match spider.on_item(item).await {
            Some(i) => i,
            None => continue,
        };
        // 再经过中间件 pipeline 链
        let item = if let Some(ref crawl_ctx) = pipeline_crawl_ctx {
            ctx.shared
                .middleware_chain
                .run_pipelines(item, crawl_ctx)
                .await
        } else {
            Some(item)
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
        if ctx.shared.follow_tx.send(f).is_err() {
            tracing::debug!("follow_tx closed, dropping follow request");
        }
    }
    // 通知主循环有新工作到来
    ctx.shared.work_notify.notify_one();

    // PageScraped 事件
    if let Some(ref tx) = ctx.state.tx {
        let status_codes_snapshot = stats.status_codes_snapshot();
        let _ = tx
            .send(CrawlEvent::PageScraped {
                url: sanitize_url(&page_url), // ND-007-SEC：事件中脱敏 URL 凭据
                stats: snapshot_stats_for(stats, status_codes_snapshot, ctx.state.start),
            })
            .await;
    }
}

// === 抓取分发 ===

/// 抓取分发：单次 fetch，**内置同步重试循环**。
///
/// 重试逻辑由 engine 统一管理（修复 ND-002-CORR：原 follow_tx 重试路径被 scheduler
/// seen 去重破坏，静默丢失重试请求）：
/// - **网络错误重试**：fetch_page 失败时，engine 在本函数内同步循环重试，
///   计数 `req.retry_count`，上限 `EngineConfig.max_retries`。
///   中间件 `RetryMiddleware` 只决定"是否重试"（业务决策），不再维护计数。
/// - **业务重做**：响应中间件 `MwAction::Refetch` 在 `process_response` 内处理，
///   计数 `refetch_depth`，上限 `EngineConfig.max_refetch_rounds`。
///
/// 两套计数器独立：`retry_count` 跨多次 fetch 失败累加，`refetch_depth` 在单次
/// process_response 内累加。互不干扰。
#[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>) {
    let stats = &ctx.state.stats;
    let max_retries = ctx.config.max_retries;
    // 同步重试循环：clone 一次，循环内修改 retry_count
    let mut current_req = req.clone();

    loop {
        let proxy = current_req.proxy.clone();
        match fetch_page(
            &ctx.config.client,
            &current_req,
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
                return (Some(resp), None);
            }
            Err(e) => {
                // Auto 模式首次连接失败：连接层拦截（TLS reset/连接拒绝/超时）
                // 无法被响应中间件（StealthUpgradeMiddleware）检测，此处主动升级
                // Stealth 重试。不消耗 retry_count（属于模式升级，非错误重试）。
                if ctx.config.fetch_mode == FetchMode::Auto
                    && current_req.fetch_mode_override.is_none()
                    && current_req.retry_count == 0
                {
                    ctx.shared
                        .rule_engine
                        .lock()
                        .await
                        .learn(&current_req.url, FetchMode::Stealth);
                    tracing::info!(
                        "AutoFallback: '{}' 首次抓取失败 ({}), 升级 Stealth 重试",
                        sanitize_url(&current_req.url),
                        e
                    );
                    current_req.fetch_mode_override = Some(FetchMode::Stealth);
                    continue;
                }

                // 调用错误中间件：只决定"是否重试"，不维护计数
                let action = if !ctx.shared.middleware_chain.is_empty() {
                    let crawl_ctx = build_crawl_context(ctx);
                    ctx.shared
                        .middleware_chain
                        .run_error_middlewares(&current_req, &e.to_string(), &crawl_ctx)
                        .await
                } else {
                    middleware::ErrorAction::Propagate
                };

                match action {
                    middleware::ErrorAction::Retry if current_req.retry_count < max_retries => {
                        current_req.retry_count += 1;
                        stats.retries.fetch_add(1, Ordering::SeqCst);
                        tracing::debug!(
                            "retry {}/{}: {}",
                            current_req.retry_count,
                            max_retries,
                            sanitize_url(&current_req.url)
                        );
                        // ND-001-ERR：发送 Retry 事件，让 stream 消费者感知重试发生
                        if let Some(ref tx) = ctx.state.tx {
                            let _ = tx.try_send(CrawlEvent::Retry {
                                url: sanitize_url(&current_req.url),
                                attempt: current_req.retry_count,
                                max: max_retries,
                                error: e.to_string(),
                            });
                        }
                        continue; // 同步重试，绕过 scheduler seen 去重
                    }
                    _ => {
                        stats.errors.fetch_add(1, Ordering::SeqCst);
                        ctx.state
                            .spider
                            .on_error(&current_req, &e.to_string())
                            .await;
                        // ND-007-SEC：错误信息中脱敏 URL 凭据，避免泄露到日志和事件
                        return (
                            None,
                            Some(format!(
                                "fetch failed: {} - {}",
                                e,
                                sanitize_url(&current_req.url)
                            )),
                        );
                    }
                }
            }
        }
    }
}

// === 辅助函数 ===

/// ND-007-SEC：脱敏 URL 中的凭据（user:pass@host）。
///
/// 错误信息和日志中不应包含 URL 凭据。此函数将 `http://user:pass@host/path`
/// 转换为 `http://***:***@host/path`，避免凭据泄露。
pub(crate) fn sanitize_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            // 无 username，直接返回原 URL
            if u.username().is_empty() {
                return url.to_string();
            }
            // 有凭据，替换为 ***
            let mut s = url.to_string();
            // 替换 username:password@ 为 ***:***@
            if let Some(at_pos) = s.find('@') {
                // 找到 scheme:// 后的位置
                if let Some(scheme_end) = s.find("://") {
                    let creds_start = scheme_end + 3;
                    if at_pos > creds_start {
                        let creds = &s[creds_start..at_pos];
                        s = s.replace(creds, "***:***");
                    }
                }
            }
            s
        }
        Err(_) => url.to_string(),
    }
}

/// ND-007-CORR：发送 Error 事件到 stream 消费者（如有 tx）。
///
/// 用于 refetch 失败、refetch 超限等场景，避免错误被静默丢弃。
/// 事件 URL 也经过脱敏处理（ND-007-SEC）。
pub(crate) fn emit_error_event(ctx: &EngineContext, url: &str, err: &str) {
    if let Some(ref tx) = ctx.state.tx {
        let _ = tx.try_send(CrawlEvent::Error {
            url: sanitize_url(url),
            error: err.to_string(),
        });
    }
}

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

/// Checkpoint 保存（从 sched + stats 序列化状态，调用底层 save_checkpoint）。
///
/// ND-003-ERR：返回 `Result<()>` 让调用方感知失败。
pub(crate) async fn persist_spider_checkpoint(
    store: &dyn crate::storage::Store,
    spider_name: &str,
    sched: &scheduler::Scheduler,
    stats: &Arc<SpiderStats>,
) -> Result<()> {
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
    let blob = bincode::serialize(&state).map_err(|e| {
        crate::error::WispError::Storage(crate::error::StorageError::General(format!(
            "checkpoint 序列化失败: {e}"
        )))
    })?;
    crate::storage::save_checkpoint(store, spider_name, &blob).map_err(|e| {
        crate::error::WispError::Storage(crate::error::StorageError::General(format!(
            "checkpoint 保存失败: {e}"
        )))
    })?;
    Ok(())
}

// === fetch_page（模式分发）===

#[doc(hidden)]
pub async fn fetch_page(
    fetch_client: &crate::fetcher::FetchClient,
    req: &Request,
    proxy_url: Option<&str>,
    mode: FetchMode,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
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
    proxy_clients: &moka::sync::Cache<String, Arc<Client>>,
) -> Result<Response> {
    // 浏览器模式：通过 BrowserPool 复用实例（RAII 自动归还，无泄漏）
    if mode == FetchMode::Dynamic || mode == FetchMode::Stealth {
        let solve_cf = mode == FetchMode::Stealth;
        let fetch_req = crate::fetcher::Request {
            url: req.url.clone(),
            method: req.method,
            headers: req.headers.clone(),
            body: req.body.clone(),
            ..Default::default()
        };
        let resp = fetch_client.fetch_browser(&fetch_req, solve_cf).await?;
        // 从 headers 提取 content_type（与 HTTP 模式一致），供 encoding 解码和缓存恢复使用
        let content_type = resp
            .headers
            .get("content-type")
            .or_else(|| resp.headers.get("Content-Type"))
            .cloned()
            .unwrap_or_default();
        return Ok(Response::from_parts(
            resp.status,
            resp.url.clone(),
            resp.headers.clone(),
            resp.body.clone(),
            resp.title.clone(),
            resp.cookies.clone(),
            req.clone(),
            content_type,
            false,
        ));
    }

    // Http 模式：统一使用 Client::fetch() 直接返回 fetcher::Response，无中间类型转换。
    let base_client = fetch_client.http();

    // 代理 Client 缓存：相同 proxy URL 复用已建立的连接，避免每请求 TLS 握手。
    // ND-009-SEC：使用 moka::sync::Cache 的 try_get_with 保证原子性插入，并限制最大条目数。
    // try_get_with 内部持锁，消除 get→insert 之间另一 task 抢先插入导致的偶发多余 Client 构建。
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        let timeout = base_client.config_ref().timeout;
        let proxy_owned = proxy.to_string();
        // moka::sync::Cache::try_get_with 返回 Result<V, Arc<E>>，需 map_err 解包 Arc
        let client: Arc<Client> = proxy_clients
            .try_get_with(proxy_owned.clone(), move || {
                let new_client = Client::builder()
                    .timeout(timeout)
                    .proxy(&proxy_owned)
                    .build()?;
                Ok::<Arc<Client>, crate::error::WispError>(Arc::new(new_client))
            })
            .map_err(|e| {
                crate::error::WispError::Network(crate::error::NetworkError::Http(format!(
                    "proxy client build failed: {}",
                    e
                )))
            })?;
        Some(client)
    } else {
        None
    };
    let use_client: &Client = match &proxy_client {
        Some(c) => c.as_ref(),
        None => base_client,
    };

    // 直接调用统一 fetch，返回的 Response 已包含 request 字段
    use_client.fetch(req).await
}

// === InFlightGuard ===

/// In-flight 计数守卫：drop 时递减计数并通知主循环。
///
/// 关键修复：drop 时必须通知 `work_notify`，否则当请求失败（process_request 返回 None，
/// process_response 不被调用）时，主循环在 scheduler 空且 in-flight > 0 的状态下
/// 会永远等待 work_notify，导致死锁。
pub(crate) struct InFlightGuard {
    pub counter: Arc<AtomicUsize>,
    pub work_notify: Arc<tokio::sync::Notify>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
        // 唤醒主循环：in-flight 计数变化，主循环需要重新检查退出条件
        self.work_notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Store;
    use async_trait::async_trait;
    use std::time::Instant;

    /// 最小 Spider：handle 返回空，不产出 items/follows，避免触碰事件通道。
    struct DummySpider;

    #[async_trait]
    impl Spider for DummySpider {
        fn name(&self) -> &str {
            "dummy"
        }
        fn start_urls(&self) -> Vec<String> {
            vec![]
        }
        async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
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
                max_retries: 3,
            },
            shared: EngineShared {
                sched: Arc::new(scheduler::Scheduler::new()),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
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
        Response::from_parts(
            200,
            "http://example.com/page".into(),
            HashMap::new(),
            vec![],
            None,
            Vec::new(),
            Request::get("http://example.com/page"),
            String::new(),
            from_cache,
        )
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

    /// Task 3：验证 persist_spider_checkpoint 把 Scheduler 的 seen_urls 集合写入持久化 blob。
    #[tokio::test]
    async fn save_checkpoint_persists_seen_urls() {
        let store = crate::storage::MemoryStore::default();
        let sched = scheduler::Scheduler::new();
        // push 两个 URL：进入 heap 与 seen 集合
        sched.push(Request::get("https://example.com/a")).await;
        sched.push(Request::get("https://example.com/b")).await;

        let stats = Arc::new(SpiderStats::new());
        persist_spider_checkpoint(&store, "seen_persist_spider", &sched, &stats)
            .await
            .expect("persist_spider_checkpoint should succeed");

        let blob = crate::storage::load_checkpoint(&store, "seen_persist_spider")
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

    /// 构造带 RetryMiddleware 的 EngineContext（max_retries 可配置）。
    fn make_ctx_with_retry(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
        let stats = Arc::new(SpiderStats::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let mut chain = middleware::MiddlewareChain::new();
        chain
            .middlewares
            .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                std::time::Duration::ZERO,
            )));

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
                max_retries,
            },
            shared: EngineShared {
                sched: Arc::new(scheduler::Scheduler::new()),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                control: Arc::new(control::EngineControl::new()),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: Arc::new(chain),
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

    /// ND-002-CORR 回归测试：fetch_dispatch 必须实际执行重试。
    ///
    /// 原实现通过 `follow_tx → sched.push` 重新入队，被 `seen_exact` 去重静默丢弃，
    /// 导致 RetryMiddleware 完全不工作。修复后 fetch_dispatch 在函数内同步循环重试，
    /// retry_count 和 stats.retries 必须实际递增。
    ///
    /// 使用不可达端口（127.0.0.1:1）触发 `connection refused`（即时返回，不依赖网络），
    /// 该错误匹配 `is_retryable_network_error`（含 "connection refused"）。
    #[tokio::test]
    async fn fetch_dispatch_actually_retries_on_network_error() {
        // max_retries=2：应重试 2 次后失败
        let (ctx, stats) = make_ctx_with_retry(2);

        // 指向不可达端口触发 connection refused
        let req = Request::get("http://127.0.0.1:1/");

        let (resp, err) = fetch_dispatch(&ctx, &req).await;

        // 重试耗尽后应返回错误（而非 Some(resp)）
        assert!(resp.is_none(), "重试耗尽后应返回 None 而非 Some(resp)");
        assert!(err.is_some(), "重试耗尽后应返回错误信息");
        let err_msg = err.unwrap();
        assert!(
            err_msg.contains("fetch failed"),
            "错误信息应含 'fetch failed'，实际: {}",
            err_msg
        );

        // 关键断言：stats.retries 必须等于 max_retries（2）
        // 原实现因 follow_tx 路径被去重，stats.retries 始终为 0
        assert_eq!(
            stats.retries.load(Ordering::SeqCst),
            2,
            "stats.retries 应等于 max_retries (2)，原 bug 导致此值为 0"
        );

        // errors 应递增 1（重试耗尽后计入一次错误）
        assert_eq!(
            stats.errors.load(Ordering::SeqCst),
            1,
            "重试耗尽后 errors 应为 1"
        );
    }

    /// ND-002-CORR 回归测试：max_retries=0 时不重试，直接失败。
    #[tokio::test]
    async fn fetch_dispatch_no_retry_when_max_retries_zero() {
        let (ctx, stats) = make_ctx_with_retry(0);

        let req = Request::get("http://127.0.0.1:1/");
        let (resp, err) = fetch_dispatch(&ctx, &req).await;

        assert!(resp.is_none(), "应返回 None");
        assert!(err.is_some(), "应返回错误");

        assert_eq!(
            stats.retries.load(Ordering::SeqCst),
            0,
            "max_retries=0 时不应重试"
        );
        assert_eq!(stats.errors.load(Ordering::SeqCst), 1, "应计入一次错误");
    }

    /// 构造 Auto 模式 EngineContext（max_concurrent_pages=0 禁用浏览器池，
    /// Stealth 模式快速返回 "browser pool not configured" 错误）。
    fn make_ctx_auto(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
        let stats = Arc::new(SpiderStats::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let mut chain = middleware::MiddlewareChain::new();
        chain
            .middlewares
            .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                std::time::Duration::ZERO,
            )));

        let fetch_config = crate::fetcher::FetchClientConfig {
            max_concurrent_pages: 0, // 禁用浏览器池，Stealth 模式快速失败
            ..Default::default()
        };

        let ctx = EngineContext {
            config: EngineConfig {
                client: Arc::new(
                    crate::fetcher::FetchClient::new(fetch_config).expect("build fetch client"),
                ),
                fetch_mode: FetchMode::Auto,
                max_concurrent: 8,
                obey_robots: false,
                engine_max_pages: 100,
                max_refetch_rounds: 5,
                max_retries,
            },
            shared: EngineShared {
                sched: Arc::new(scheduler::Scheduler::new()),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                control: Arc::new(control::EngineControl::new()),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: Arc::new(chain),
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

    /// Auto 模式首次连接失败时，fetch_dispatch 应主动升级 Stealth 重试。
    ///
    /// 连接层拦截（TLS reset/连接拒绝）无法被响应中间件 StealthUpgradeMiddleware
    /// 检测（因为没有 HTTP 响应）。fetch_dispatch 在错误处理中主动升级：
    /// 1. HTTP 模式失败 → learn(rule_engine, Stealth) + set override + continue
    /// 2. Stealth 模式失败（此处 browser pool 未配置）→ 走正常错误流程
    ///
    /// 验证：rule_engine 学到了该 URL 需要 Stealth（resolve 返回 Some）。
    #[tokio::test]
    async fn fetch_dispatch_auto_upgrades_to_stealth_on_first_failure() {
        let (ctx, _stats) = make_ctx_auto(0);

        let url = "http://127.0.0.1:1/auto-upgrade-test";
        let req = Request::get(url);
        let (resp, err) = fetch_dispatch(&ctx, &req).await;

        // Stealth 模式也失败（browser pool 未配置），最终返回错误
        assert!(resp.is_none(), "Stealth 也失败时应返回 None");
        assert!(err.is_some(), "应返回错误信息");

        // 关键断言：rule_engine 应学到该 URL 需要 Stealth
        let resolved = ctx.shared.rule_engine.lock().await.resolve(url);
        assert_eq!(
            resolved,
            Some(FetchMode::Stealth),
            "Auto 模式首次失败后 rule_engine 应学到 Stealth，实际: {:?}",
            resolved
        );
    }

    // === ND-012-TEST：核心函数补充测试 ===
    //
    // 覆盖 check_control_and_hook 控制流、process_request 错误路径、
    // sanitize_url 凭据脱敏、CrawlEvent 发送。

    /// cancel(url) 后 check_control_and_hook 应返回 false（跳过请求）。
    #[tokio::test]
    async fn check_control_cancelled_url_returns_false() {
        let (ctx, _stats) = make_ctx();
        let url = "http://example.com/cancelled";
        ctx.shared.control.cancel(url).await;
        let req = Request::get(url);
        assert!(
            !check_control_and_hook(&ctx, &req).await,
            "cancelled URL 应返回 false"
        );
    }

    /// shutdown 后 check_control_and_hook 应返回 false。
    #[tokio::test]
    async fn check_control_shutdown_returns_false() {
        let (ctx, _stats) = make_ctx();
        ctx.shared.control.shutdown();
        let req = Request::get("http://example.com/any");
        assert!(
            !check_control_and_hook(&ctx, &req).await,
            "shutdown 后应返回 false"
        );
    }

    /// pause(url) + shutdown 应返回 false（不死锁，由 wait_if_paused 检测 shutdown 退出）。
    #[tokio::test]
    async fn check_control_pause_then_shutdown_returns_false() {
        let (ctx, _stats) = make_ctx();
        let url = "http://example.com/paused";
        // 先 pause，再 shutdown（避免 wait_if_paused 永久阻塞）
        ctx.shared.control.pause(url).await;
        ctx.shared.control.shutdown();
        let req = Request::get(url);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            check_control_and_hook(&ctx, &req),
        )
        .await;
        assert!(result.is_ok(), "pause+shutdown 不应死锁");
        assert!(!result.unwrap(), "pause+shutdown 后应返回 false");
    }

    /// cancelled URL 时 process_request 应返回 None（不发起抓取）。
    #[tokio::test]
    async fn process_request_cancelled_url_returns_none() {
        let (ctx, stats) = make_ctx();
        let url = "http://example.com/cancelled";
        ctx.shared.control.cancel(url).await;
        let req = Request::get(url);
        let result = process_request(&ctx, req).await;
        assert!(result.is_none(), "cancelled URL 应返回 None");
        // pages 不应递增（未抓取）
        assert_eq!(stats.pages.load(Ordering::SeqCst), 0);
    }

    /// shutdown 时 process_request 应返回 None。
    #[tokio::test]
    async fn process_request_shutdown_returns_none() {
        let (ctx, _stats) = make_ctx();
        ctx.shared.control.shutdown();
        let req = Request::get("http://example.com/any");
        let result = process_request(&ctx, req).await;
        assert!(result.is_none(), "shutdown 时应返回 None");
    }

    /// 构造带事件通道的 EngineContext，返回 (ctx, stats, rx) 用于消费事件。
    fn make_ctx_with_tx(
        max_retries: u32,
    ) -> (
        EngineContext,
        Arc<SpiderStats>,
        tokio::sync::mpsc::Receiver<CrawlEvent>,
    ) {
        let stats = Arc::new(SpiderStats::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
        let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
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
                max_retries,
            },
            shared: EngineShared {
                sched: Arc::new(scheduler::Scheduler::new()),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                control: Arc::new(control::EngineControl::new()),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: Arc::new({
                    let mut chain = middleware::MiddlewareChain::new();
                    chain
                        .middlewares
                        .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                            std::time::Duration::ZERO,
                        )));
                    chain
                }),
                rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
            },
            state: EngineState {
                spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                stats: stats.clone(),
                items: Arc::new(Mutex::new(Vec::new())),
                abort_flag: Arc::new(AtomicBool::new(false)),
                start: Instant::now(),
                tx: Some(tx),
                global_in_flight: Arc::new(AtomicUsize::new(0)),
            },
        };
        (ctx, stats, rx)
    }

    /// fetch 失败重试耗尽后应发送 CrawlEvent::Error。
    #[tokio::test]
    async fn process_request_emits_error_event_on_failure() {
        let (ctx, _stats, mut rx) = make_ctx_with_tx(1);
        let req = Request::get("http://127.0.0.1:1/");
        let result = process_request(&ctx, req).await;
        assert!(result.is_none(), "失败后应返回 None");

        // 应收到 Error 事件
        let mut got_error = false;
        while let Ok(Some(event)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
        {
            if matches!(event, CrawlEvent::Error { .. }) {
                got_error = true;
                break;
            }
        }
        assert!(got_error, "应收到 CrawlEvent::Error 事件");
    }

    /// 重试期间应发送 CrawlEvent::Retry 事件（max_retries=2 时至少 1 次 Retry）。
    #[tokio::test]
    async fn process_request_emits_retry_events() {
        let (ctx, _stats, mut rx) = make_ctx_with_tx(2);
        let req = Request::get("http://127.0.0.1:1/");
        let _ = process_request(&ctx, req).await;

        // 收集所有事件
        let mut retry_count = 0;
        let mut got_error = false;
        while let Ok(Some(event)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
        {
            match event {
                CrawlEvent::Retry { attempt, max, .. } => {
                    assert_eq!(max, 2, "Retry 事件 max 应为 2");
                    assert!(attempt >= 1, "attempt 应 >= 1");
                    retry_count += 1;
                }
                CrawlEvent::Error { .. } => {
                    got_error = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(retry_count >= 1, "应至少发送 1 次 Retry 事件");
        assert!(got_error, "重试耗尽后应发送 Error 事件");
    }

    /// sanitize_url：无凭据的 URL 应原样返回。
    #[test]
    fn sanitize_url_no_credentials() {
        let url = "https://example.com/path?q=1";
        assert_eq!(sanitize_url(url), url);
    }

    /// sanitize_url：有凭据的 URL 应脱敏为 ***:***@host。
    #[test]
    fn sanitize_url_redacts_credentials() {
        let url = "http://user:pass@example.com/path";
        let sanitized = sanitize_url(url);
        assert!(!sanitized.contains("user"), "脱敏后不应包含 username");
        assert!(!sanitized.contains("pass"), "脱敏后不应包含 password");
        assert!(
            sanitized.contains("***:***@"),
            "脱敏后应包含 ***:***@，实际: {}",
            sanitized
        );
        assert!(sanitized.contains("example.com"), "脱敏后应保留 host");
    }

    /// sanitize_url：解析失败的 URL 应原样返回（不 panic）。
    #[test]
    fn sanitize_url_invalid_url_passthrough() {
        let url = "not a url at all";
        assert_eq!(sanitize_url(url), url);
    }

    /// sanitize_url：只有 username 没有 password的 URL 也应脱敏。
    #[test]
    fn sanitize_url_only_username() {
        let url = "http://user@example.com/";
        let sanitized = sanitize_url(url);
        assert!(!sanitized.contains("user@"));
        assert!(sanitized.contains("***"));
    }

    /// record_status：状态码计数应正确累加。
    #[test]
    fn record_status_increments_counter() {
        let stats = Arc::new(SpiderStats::new());
        record_status(&stats, 200);
        record_status(&stats, 200);
        record_status(&stats, 404);
        let snapshot = stats.status_codes_snapshot();
        assert_eq!(snapshot.get(&200).copied(), Some(2));
        assert_eq!(snapshot.get(&404).copied(), Some(1));
    }

    /// snapshot_stats_for：应正确填充统计快照字段。
    #[test]
    fn snapshot_stats_for_populates_fields() {
        let stats = Arc::new(SpiderStats::new());
        stats.pages.store(10, Ordering::SeqCst);
        stats.items.store(50, Ordering::SeqCst);
        stats.errors.store(2, Ordering::SeqCst);
        stats.retries.store(5, Ordering::SeqCst);
        let snapshot = snapshot_stats_for(&stats, HashMap::new(), Instant::now());
        assert_eq!(snapshot.pages_crawled, 10);
        assert_eq!(snapshot.items_scraped, 50);
        assert_eq!(snapshot.errors, 2);
        assert_eq!(snapshot.retry_count, 5);
    }
}
