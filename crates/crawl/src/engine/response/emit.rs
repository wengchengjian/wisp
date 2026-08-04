//! Engine 子模块：response events and item delivery.

use super::*;
use crate::Item;
use wisp_core::error::Result;

pub(super) async fn process_page_items(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
    source_url: &str,
    callback: Option<&str>,
    items: Vec<Value>,
) -> Result<()> {
    let pipeline_crawl_ctx = if ctx.state.middleware_chain.is_empty() {
        None
    } else {
        Some(build_crawl_context_for(ctx, spider, stats))
    };
    for item in items {
        let item = match spider.on_item(item).await {
            Some(i) => i,
            None => continue,
        };
        let item = Item::new(item, source_url, spider.name(), callback);
        let item = if let Some(ref crawl_ctx) = pipeline_crawl_ctx {
            ctx.state
                .middleware_chain
                .run_pipelines(item, crawl_ctx)
                .await
        } else {
            Ok(Some(item))
        };
        let processed = match item {
            Ok(processed) => processed,
            Err(e) => {
                let msg = e.to_string();
                emit_error_event(ctx, source_url, &msg).await;
                *ctx.state.pipeline_error.lock().await = Some(e);
                ctx.state.abort_flag.store(true, Ordering::SeqCst);
                ctx.state.work_notify.notify_waiters();
                return Err(wisp_core::error::WispError::Engine(format!(
                    "item pipeline failed: {msg}"
                )));
            }
        };
        if let Some(processed) = processed {
            stats.items.fetch_add(1, Ordering::SeqCst);
            ctx.runtime
                .event_bus
                .emit(CrawlEvent::Item(processed))
                .await;
        }
    }
    Ok(())
}

/// 将 follow 请求送入主循环队列。
pub(super) async fn schedule_follow_requests(ctx: &EngineContext, follows: Vec<CrawlRequest>) {
    for f in follows {
        if ctx.state.follow_tx.send(f).is_err() {
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
    ctx.runtime
        .event_bus
        .emit(CrawlEvent::PageScraped {
            url: sanitize_url(page_url),
            stats: stats.snapshot(),
        })
        .await;
}

pub(super) async fn emit_error_event(ctx: &EngineContext, url: &str, err: &str) {
    ctx.runtime
        .event_bus
        .emit(CrawlEvent::Error {
            url: sanitize_url(url),
            error: err.to_string(),
            attempt: 0,
        })
        .await;
}
