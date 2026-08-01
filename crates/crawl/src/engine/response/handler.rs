//! Engine 子模块：response handler。

use super::emit::{
    emit_error_event, emit_page_scraped, process_page_items, schedule_follow_requests,
};
use super::middleware::{apply_response_middlewares, maybe_persist_checkpoint};
use super::*;

async fn handle_spider_page(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    resp: Response,
) -> (Vec<Value>, Vec<Request>) {
    let page_url = resp.url.clone();
    let status = resp.status;
    let spider_for_handle = Arc::clone(spider);
    let handle_task = tokio::spawn(async move {
        spider_for_handle
            .handle(resp)
            .instrument(tracing::trace_span!(
                "spider.handle",
                spider = %spider_for_handle.name()
            ))
            .await
    });
    match handle_task.await {
        Ok(result) => {
            log_handle_result(&page_url, status, &result.0, &result.1);
            result
        }
        Err(join_err) => {
            tracing::error!(
                "spider.handle panic/abort: {} - {}",
                sanitize_url(&page_url),
                join_err
            );
            emit_error_event(
                ctx,
                &page_url,
                &format!("spider.handle panic/abort: {join_err}"),
            )
            .await;
            (Vec::new(), Vec::new())
        }
    }
}

fn log_handle_result(url: &str, status: u16, items: &[Value], follows: &[Request]) {
    if items.is_empty() && follows.is_empty() {
        tracing::warn!(
            "handle 返回空 (items=0, follows=0): url={}, status={}",
            sanitize_url(url),
            status
        );
    } else {
        tracing::info!(
            "handle 完成: url={}, items={}, follows={}",
            sanitize_url(url),
            items.len(),
            follows.len()
        );
    }
}

/// 处理已获取的响应：handle → Auto 升级 → items → events。
///
/// Task 3 关键改动：调用 `spider.handle(resp)`（callback 路由）而非 `spider.parse(resp)`。
/// items 同时收集到 `ctx.items`（供 `Engine::run` 返回）和 `tx`（供 `run_stream` 消费）。
#[tracing::instrument(level = "trace", skip(ctx, resp), fields(status = resp.status))]

pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
    let spider = match ctx.state.spider_for(&resp.request) {
        Some(s) => s,
        None => return,
    };
    let stats = ctx.state.stats_for(&resp.request).expect("spider stats");

    if !resp.from_cache {
        stats.pages.fetch_add(1, Ordering::SeqCst);
        let callback = resp.request.callback.as_deref().unwrap_or("default");
        stats.record_callback_page(callback);
    }
    maybe_persist_checkpoint(ctx, &spider, &stats).await;

    let page_url = resp.url.clone();
    let resp = if ctx.shared.middleware_chain.is_empty() {
        Some(resp)
    } else {
        apply_response_middlewares(ctx, &spider, &stats, &page_url, resp).await
    };
    let Some(resp) = resp else {
        return;
    };

    let (items, follows) = handle_spider_page(ctx, &spider, resp).await;
    process_page_items(ctx, &spider, &stats, &page_url, items).await;
    schedule_follow_requests(ctx, follows).await;
    ctx.shared.work_notify.notify_one();
    emit_page_scraped(ctx, &stats, &page_url).await;
}
