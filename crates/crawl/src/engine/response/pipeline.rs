//! Engine 子模块：item pipeline 编排。
//!
//! 负责将 Spider 产出的 items 经 `on_item` → `Item::new` →
//! `middleware_chain.run_pipelines` 编排后交付。pipeline 失败时
//! 发错误事件并中止 run。

use super::*;
use crate::Item;
use wisp_core::error::Result;

/// 处理 Spider 产出的 items：on_item → pipeline → 事件交付。
///
/// pipeline 返回 `Err` 时发 `CrawlEvent::Error`，设置 `abort_flag` 与
/// `pipeline_error`，唤醒等待的 worker，并返回typed 错误。
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
                emit::emit_error_event(ctx, source_url, &msg).await;
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
