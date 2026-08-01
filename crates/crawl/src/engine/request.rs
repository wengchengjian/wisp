//! Engine 子模块：request。

use super::*;

// === 核心函数：处理单个请求 ===

/// Stage 1: 控制状态检查 + Spider 钩子（基础设施级，不可中间件化）。
pub(crate) async fn check_control_and_hook(
    ctx: &EngineContext,
    req: &Request,
    spider: &Arc<dyn Spider>,
) -> bool {
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
    match spider.on_before_request(req).await {
        crate::RequestAction::Proceed => true,
        crate::RequestAction::Skip => false,
        crate::RequestAction::Delay(d) => {
            tokio::time::sleep(d).await;
            true
        }
        crate::RequestAction::Abort => {
            ctx.state.abort_flag.store(true, Ordering::SeqCst);
            false
        }
    }
}

/// 处理请求阶段：控制检查 → 中间件请求链 → 抓取。

/// Spider 业务策略过滤：域名白名单与最大深度。
fn is_allowed_domain(spider: &Arc<dyn Spider>, req: &Request) -> bool {
    if spider.allowed_domains().is_empty() {
        return true;
    }
    let host = url::Url::parse(&req.url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();
    spider
        .allowed_domains()
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// 请求阶段中间件结果：继续抓取、直接响应（缓存）或停止。
enum RequestStage {
    Continue,
    Respond(Response),
    Stop,
}

/// 执行请求中间件链，返回是否继续、缓存响应或终止。
async fn run_request_middlewares(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
    req: &mut Request,
) -> RequestStage {
    let crawl_ctx = build_crawl_context_for(ctx, spider, stats);
    match ctx
        .shared
        .middleware_chain
        .run_request_middlewares(req, &crawl_ctx)
        .await
    {
        middleware::RequestMwAction::Skip => RequestStage::Stop,
        middleware::RequestMwAction::Abort(reason) => {
            tracing::warn!("middleware abort: {} - {}", reason, sanitize_url(&req.url));
            RequestStage::Stop
        }
        middleware::RequestMwAction::Respond(cached_resp) => {
            stats.cache_hits.fetch_add(1, Ordering::SeqCst);
            record_status(stats, cached_resp.status);
            ctx.shared
                .event_bus
                .emit(EngineEvent::ResponseReceived {
                    url: sanitize_url(&req.url),
                    status: cached_resp.status,
                    elapsed_ms: 0,
                    from_cache: true,
                })
                .await;
            RequestStage::Respond(cached_resp)
        }
        middleware::RequestMwAction::Continue | middleware::RequestMwAction::Modified => {
            RequestStage::Continue
        }
    }
}

/// 发送抓取失败事件（stream 与 event bus），保持错误分类在调用链内。
async fn emit_fetch_failure(ctx: &EngineContext, req: &Request, e: &wisp_core::error::WispError) {
    if let Some(ref tx) = ctx.state.tx {
        let _ = tx
            .send(CrawlEvent::Error {
                url: sanitize_url(&req.url),
                error: format!("fetch failed: {e} - {}", sanitize_url(&req.url)),
            })
            .await;
    }
    ctx.shared
        .event_bus
        .emit(EngineEvent::ErrorOccurred {
            url: sanitize_url(&req.url),
            error: e.to_string(),
            attempt: req.retry_count,
        })
        .await;
}
///
/// 返回 `Some(resp)` 表示请求阶段产出响应，需由调用方交给 `process_response` 处理；
/// 返回 `None` 表示已处理完毕（Skip/Abort/错误已发送事件），无需后续。
///
/// Stages:
/// 1. 控制状态 + Spider 钩子（基础设施）
/// 2. 中间件请求链（域名/深度/robots/缓存/延迟/UA/代理 全部在此）
/// 3. 域名信号量（并发控制）+ fetch
#[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
/// 处理请求阶段：控制检查 → 中间件请求链 → 抓取。
///
/// 返回 `Some(resp)` 表示请求阶段产出响应，需由调用方交给 `process_response` 处理；
/// 返回 `None` 表示已处理完毕（Skip/Abort/错误已发送事件），无需后续。
#[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
pub(crate) async fn process_request(ctx: &EngineContext, req: Request) -> Option<Response> {
    let spider = ctx.state.spider_for(&req)?;
    let stats = ctx.state.stats_for(&req)?;

    if !is_allowed_domain(&spider, &req) {
            stats.offsite.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        if req.depth > spider.max_depth() {
            return None;
        }
    if !check_control_and_hook(ctx, &req, &spider).await {
        return None;
    }

    let mut req = req;
    if !ctx.shared.middleware_chain.is_empty() {
        match run_request_middlewares(ctx, &spider, &stats, &mut req).await {
            RequestStage::Stop => return None,
            RequestStage::Respond(resp) => return Some(resp),
            RequestStage::Continue => {}
        }
    }

    let fetch_started = std::time::Instant::now();
    match fetch_dispatch(ctx, &req).await {
        Ok(resp) => {
            ctx.shared
                .event_bus
                .emit(EngineEvent::ResponseReceived {
                    url: sanitize_url(&req.url),
                    status: resp.status,
                    elapsed_ms: fetch_started.elapsed().as_millis() as u64,
                    from_cache: resp.from_cache,
                })
                .await;
            Some(resp)
        }
        Err(e) => {
            emit_fetch_failure(ctx, &req, &e).await;
            None
        }
    }
}
