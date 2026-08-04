//! Engine 子模块：事件发射原语。
//!
//! 提供 `PageScraped` 与 `Error` 事件的发射函数，供 handler 与 pipeline
//! 模块调用。本模块只负责"把事件送到 event_bus"，不编排业务逻辑。

use super::*;

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

/// 发送 Error 事件。
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
