//! Engine 子模块：response events and item collection。

use super::*;

pub(super) async fn process_page_items(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
    page_url: &str,
    items: Vec<Value>,
) {
    let pipeline_crawl_ctx = if ctx.shared.middleware_chain.is_empty() {
        None
    } else {
        Some(build_crawl_context_for(ctx, spider, stats))
    };
    for item in items {
        let item = match spider.on_item(item).await {
            Some(i) => i,
            None => continue,
        };
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
            ctx.shared
                .event_bus
                .emit(EngineEvent::ItemScraped {
                    url: sanitize_url(page_url),
                })
                .await;
            if let Some(ref tx) = ctx.state.tx {
                let _ = tx.send(CrawlEvent::Item(processed.clone())).await;
            }
            ctx.state.items.lock().await.push(processed);
        }
    }
}

/// 将 follow 请求送入主循环队列并发送调度事件。
pub(super) async fn schedule_follow_requests(ctx: &EngineContext, follows: Vec<Request>) {
    for f in follows {
        ctx.shared
            .event_bus
            .emit(EngineEvent::RequestScheduled {
                url: sanitize_url(&f.url),
                depth: f.depth,
            })
            .await;
        if ctx.shared.follow_tx.send(f).is_err() {
            tracing::debug!("follow_tx closed, dropping follow request");
        }
    }
}

/// 发送 PageScraped 事件（含当前 stats 快照）。
pub(super) async fn emit_page_scraped(
    ctx: &EngineContext,
    stats: &Arc<SpiderStats>,
    page_url: &str,
) {
    if let Some(ref tx) = ctx.state.tx {
        let status_codes_snapshot = stats.status_codes_snapshot();
        let _ = tx
            .send(CrawlEvent::PageScraped {
                url: sanitize_url(page_url),
                stats: snapshot_stats_for(stats, status_codes_snapshot),
            })
            .await;
    }
}

pub(super) async fn emit_error_event(ctx: &EngineContext, url: &str, err: &str) {
    if let Some(ref tx) = ctx.state.tx {
        let _ = tx
            .send(CrawlEvent::Error {
                url: sanitize_url(url),
                error: err.to_string(),
            })
            .await;
    }
    ctx.shared
        .event_bus
        .emit(EngineEvent::ErrorOccurred {
            url: sanitize_url(url),
            error: err.to_string(),
            attempt: 0,
        })
        .await;
}
