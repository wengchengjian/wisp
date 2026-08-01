//! Engine 子模块：response middleware。

use super::emit::emit_error_event;
use super::*;
use crate::middleware;

pub(super) async fn maybe_persist_checkpoint(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
) {
    let Some(store) = &ctx.config.checkpoint_store else {
        return;
    };
    let interval = ctx.config.checkpoint_interval.max(1);
    let pages = stats.pages.load(Ordering::SeqCst);
    if pages == 0 || pages % interval != 0 {
        return;
    }
    let in_flight = ctx
        .state
        .in_flight_requests
        .lock()
        .await
        .get(spider.name())
        .cloned()
        .unwrap_or_default();
    match persist_spider_checkpoint(
        store.as_ref(),
        spider.name(),
        &ctx.shared.sched,
        stats,
        in_flight,
    )
    .await
    {
        Ok(()) => {
            ctx.shared
                .event_bus
                .emit(EngineEvent::CheckpointSaved {
                    pending: ctx.shared.sched.len().await,
                })
                .await;
        }
        Err(e) => tracing::warn!("checkpoint 保存失败: {e}"),
    }
}

/// 响应中间件链，支持最多 `max_refetch_rounds` 轮 Refetch。

async fn refetch_or_error(
    ctx: &EngineContext,
    new_req: &Request,
    max_rounds: u32,
    refetch_depth: u32,
) -> Option<Response> {
    if refetch_depth > max_rounds {
        let msg = format!("refetch exceeded {} rounds limit", max_rounds);
        tracing::warn!(
            "Refetch 超过 {} 轮上限，放弃: {}",
            max_rounds,
            sanitize_url(&new_req.url)
        );
        emit_error_event(ctx, &new_req.url, &msg).await;
        return None;
    }
    tracing::debug!(
        "中间件 Refetch (round {}): {}",
        refetch_depth,
        sanitize_url(&new_req.url)
    );
    match fetch_dispatch(ctx, new_req).await {
        Ok(r) => Some(r),
        Err(e) => {
            let err_msg = format!("refetch failed: {e}");
            tracing::warn!(
                "Refetch 失败，放弃: {} - {}",
                sanitize_url(&new_req.url),
                err_msg
            );
            emit_error_event(ctx, &new_req.url, &err_msg).await;
            None
        }
    }
}

pub(super) async fn apply_response_middlewares(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
    page_url: &str,
    mut resp: Response,
) -> Option<Response> {
    let mut refetch_depth = 0u32;
    loop {
        let crawl_ctx = build_crawl_context_for(ctx, spider, stats);
        match ctx
            .shared
            .middleware_chain
            .run_response_middlewares(&mut resp, &crawl_ctx)
            .await
        {
            middleware::ResponseMwAction::Skip => return None,
            middleware::ResponseMwAction::Abort(reason) => {
                tracing::warn!(
                    "response middleware abort: {} - {}",
                    reason,
                    sanitize_url(page_url)
                );
                return None;
            }
            middleware::ResponseMwAction::Refetch(new_req) => {
                refetch_depth += 1;
                let Some(r) = refetch_or_error(
                    ctx,
                    &new_req,
                    ctx.config.max_refetch_rounds as u32,
                    refetch_depth,
                )
                .await
                else {
                    return None;
                };
                resp = r;
            }
            middleware::ResponseMwAction::Continue | middleware::ResponseMwAction::Modified => {
                return Some(resp);
            }
        }
    }
}
