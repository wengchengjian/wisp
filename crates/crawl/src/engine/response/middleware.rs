//! Engine 子模块：response middleware。

use super::emit::emit_error_event;
use super::*;
use crate::middleware;

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
            .state
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
                let r = refetch_or_error(
                    ctx,
                    &new_req,
                    ctx.config.max_refetch_rounds as u32,
                    refetch_depth,
                )
                .await?;
                resp = r;
            }
            middleware::ResponseMwAction::Continue | middleware::ResponseMwAction::Modified => {
                return Some(resp);
            }
        }
    }
}
