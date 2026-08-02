//! Engine 子模块：fetch dispatch。

use super::page::fetch_page;
use super::*;

// === 抓取分发 ===

/// Auto 模式连接层失败且规则引擎尚未学习 Stealth 时，触发首次升级。
async fn should_auto_upgrade(ctx: &EngineContext, req: &Request) -> bool {
    if ctx.config.fetch_mode != FetchMode::Auto
        || req.fetch_mode_override.is_some()
        || req.retry_count != 0
    {
        return false;
    }
    ctx.state.rule_engine.lock().await.resolve(&req.url) != Some(FetchMode::Stealth)
}

/// 学习 Stealth 规则并构造带模式覆盖的重试请求。
async fn build_auto_upgrade_request(
    ctx: &EngineContext,
    req: &Request,
    e: &wisp_core::error::WispError,
) -> Request {
    ctx.state
        .rule_engine
        .lock()
        .await
        .learn(&req.url, FetchMode::Stealth);
    ctx.runtime
        .event_bus
        .emit(EngineEvent::AutoUpgraded {
            url: sanitize_url(&req.url),
            from: FetchMode::Http,
            to: FetchMode::Stealth,
        })
        .await;
    tracing::info!(
        "AutoFallback: '{}' 首次抓取失败 ({}), 升级 Stealth 重试",
        sanitize_url(&req.url),
        e
    );
    let mut upgraded = req.clone();
    upgraded.fetch_mode_override = Some(FetchMode::Stealth);
    upgraded
}

/// 运行错误中间件，决定是否重试。
async fn run_error_middleware(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
    req: &Request,
    e: &wisp_core::error::WispError,
) -> middleware::ErrorAction {
    if ctx.state.middleware_chain.is_empty() {
        return middleware::ErrorAction::Propagate;
    }
    let crawl_ctx = build_crawl_context_for(ctx, spider, stats);
    ctx.state
        .middleware_chain
        .run_error_middlewares(req, e, &crawl_ctx)
        .await
}

/// 构造重试请求并发送 Retry/Error 事件；达到上限时返回 `None`。
async fn emit_retry_request(
    ctx: &EngineContext,
    stats: &Arc<SpiderStats>,
    req: &Request,
    max_retries: u32,
    e: &wisp_core::error::WispError,
) -> Option<Request> {
    if req.retry_count >= max_retries {
        return None;
    }
    let retry_count = req.retry_count + 1;
    let url_for_log = sanitize_url(&req.url);
    let mut retried = req.clone();
    retried.retry_count = retry_count;
    stats.retries.fetch_add(1, Ordering::SeqCst);
    tracing::debug!("retry {}/{}", retry_count, max_retries);
    if let Some(ref tx) = ctx.state.tx {
        let _ = tx
            .send(CrawlEvent::Retry {
                url: url_for_log.clone(),
                attempt: retry_count,
                max: max_retries,
                error: e.to_string(),
            })
            .await;
    }
    ctx.runtime
        .event_bus
        .emit(EngineEvent::ErrorOccurred {
            url: url_for_log,
            error: e.to_string(),
            attempt: retry_count,
        })
        .await;
    Some(retried)
}

/// 记录成功响应并检测封锁。
async fn record_fetch_success(
    ctx: &EngineContext,
    stats: &Arc<SpiderStats>,
    spider: &Arc<dyn Spider>,
    resp: &Response,
) {
    record_status(stats, resp.status);
    if !spider.is_blocked(resp) {
        return;
    }
    stats.blocked.fetch_add(1, Ordering::SeqCst);
    ctx.runtime
        .event_bus
        .emit(EngineEvent::BlockedDetected {
            url: sanitize_url(&resp.request.url),
            status: resp.status,
        })
        .await;
}

/// 抓取分发：单次 fetch，**内置同步重试循环**。
///
/// 重试逻辑由 engine 统一管理（修复 ND-002-CORR：原 follow_tx 重试路径被 scheduler
/// seen 去重破坏，静默丢失重试请求）：
/// - **网络错误重试**：fetch_page 失败时，engine 在本函数内同步循环重试，
///   计数 `req.retry_count`，上限 `EngineConfig.max_retries`。
///   中间件 `RetryMiddleware` 只决定"是否重试"（业务决策），不再维护计数。
/// - **业务重做**：响应中间件 `ResponseMwAction::Refetch` 在 `process_response` 内处理，
///   计数 `refetch_depth`，上限 `EngineConfig.max_refetch_rounds`。
///
/// 两套计数器独立：`retry_count` 跨多次 fetch 失败累加，`refetch_depth` 在单次
/// process_response 内累加。互不干扰。
#[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
pub(crate) async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> Result<Response> {
    let stats = ctx.state.stats_for(req).ok_or_else(|| {
        wisp_core::error::WispError::Engine("request has no matching spider".into())
    })?;
    let spider = ctx.state.spider_for(req).ok_or_else(|| {
        wisp_core::error::WispError::Engine("request has no matching spider".into())
    })?;
    let max_retries = ctx.config.max_retries;
    let mut owned: Option<Request> = None;

    loop {
        let req_ref = owned.as_ref().unwrap_or(req);
        match fetch_page(
            &ctx.runtime.fetch_client,
            req_ref,
            ctx.config.fetch_mode,
            &ctx.state.rule_engine,
            &ctx.state.cf_domain_locks,
        )
        .await
        {
            Ok(resp) => {
                record_fetch_success(ctx, &stats, &spider, &resp).await;
                return Ok(resp);
            }
            Err(e) => {
                if should_auto_upgrade(ctx, req_ref).await {
                    owned = Some(build_auto_upgrade_request(ctx, req_ref, &e).await);
                    continue;
                }
                let action = run_error_middleware(ctx, &spider, &stats, req_ref, &e).await;
                if matches!(action, middleware::ErrorAction::Retry) {
                    if let Some(retried) =
                        emit_retry_request(ctx, &stats, req_ref, max_retries, &e).await
                    {
                        owned = Some(retried);
                        continue;
                    }
                }
                stats.errors.fetch_add(1, Ordering::SeqCst);
                spider.on_error(req_ref, &e.to_string()).await;
                return Err(e);
            }
        }
    }
}
