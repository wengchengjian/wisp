//! 调度决策与任务选取。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{NextWork, NextWorkResult};
use crate::CrawlRequest;
use crate::Spider;
use crate::engine;
use crate::stats::SpiderStats;
use crate::stop;
use wisp_core::utils::sanitize_url;

async fn drain_follow_queue(ctx: &engine::EngineContext) {
    let mut rx_guard = ctx.state.queue.follow_rx.lock().await;
    while let Ok(req) = rx_guard.try_recv() {
        ctx.state.queue.sched.push(req).await;
    }
}

/// 将 follow 请求送入主循环队列（与 [`drain_follow_queue`] 配对）。
///
/// `drain_follow_queue` 消费 `follow_rx`，本函数生产 `follow_tx`。
/// 发送失败（接收端关闭）时仅记录 debug 日志，不阻断流程。
pub(crate) async fn schedule_follow_requests(
    ctx: &engine::EngineContext,
    follows: Vec<CrawlRequest>,
) {
    for f in follows {
        if ctx.state.queue.follow_tx.send(f).is_err() {
            tracing::debug!("follow_tx closed, dropping follow request");
        }
    }
}

fn stop_context_for(
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
    queue_size: usize,
) -> stop::StopContext {
    let until = spider.until();
    stop::StopContext {
        pages: stats.pages.load(Ordering::SeqCst),
        items: stats.items.load(Ordering::SeqCst),
        errors: stats.errors.load(Ordering::SeqCst),
        in_flight: stats.in_flight.load(Ordering::SeqCst),
        elapsed: stats.start.elapsed(),
        queue_size,
        callback_pages: if until.uses_callback_pages() {
            stats.callback_pages_snapshot()
        } else {
            HashMap::new()
        },
    }
}

fn done_or_wait(in_flight: usize) -> NextWorkResult {
    if in_flight == 0 {
        NextWorkResult::Done
    } else {
        NextWorkResult::Wait
    }
}

async fn scheduling_decision(
    ctx: &engine::EngineContext,
    autoscale: Option<&crate::runtime::autoscale::AutoscaledPool>,
) -> Option<NextWorkResult> {
    if ctx.state.run.abort_flag.load(Ordering::SeqCst) {
        return Some(NextWorkResult::Done);
    }
    if ctx.runtime.control.is_shutdown() {
        return Some(done_or_wait(
            ctx.state.run.global_in_flight.load(Ordering::SeqCst),
        ));
    }
    drain_follow_queue(ctx).await;
    let total_pages: usize = ctx
        .state
        .spiders
        .all_stats
        .iter()
        .map(|s| s.pages.load(Ordering::SeqCst))
        .sum();
    if total_pages + ctx.state.run.global_in_flight.load(Ordering::SeqCst) >= ctx.config.max_pages {
        return Some(done_or_wait(
            ctx.state.run.global_in_flight.load(Ordering::SeqCst),
        ));
    }
    let limit = match autoscale {
        Some(pool) => pool.current_concurrency(),
        None => ctx.config.max_concurrent,
    };
    if ctx.state.run.global_in_flight.load(Ordering::SeqCst) >= limit {
        return Some(NextWorkResult::Wait);
    }
    None
}

pub(super) async fn next_work(
    ctx: &engine::EngineContext,
    autoscale: Option<&crate::runtime::autoscale::AutoscaledPool>,
) -> NextWorkResult {
    if let Some(result) = scheduling_decision(ctx, autoscale).await {
        return result;
    }
    let queue_size = ctx.state.queue.sched.len().await;
    let mut req = match ctx.state.queue.sched.pop().await {
        Some(req) => req,
        None => return done_or_wait(ctx.state.run.global_in_flight.load(Ordering::SeqCst)),
    };
    let Some(idx) = ctx.state.spiders.spider_index_for(&req) else {
        tracing::warn!("丢弃无 Spider 接收的请求: url={}", sanitize_url(&req.url));
        return NextWorkResult::Continue;
    };
    let spider = Arc::clone(&ctx.state.spiders.spiders()[idx]);
    let stats = Arc::clone(&ctx.state.spiders.all_stats[idx]);
    tracing::debug!(
        spider = spider.name(),
        callback = ?req.callback,
        url = %sanitize_url(&req.url),
        "next_work：请求路由到 spider"
    );
    let stop_ctx = stop_context_for(&spider, &stats, queue_size);
    if spider.until().should_stop(&stop_ctx) {
        tracing::info!(
            "Spider '{}' until() 触发，丢弃后续请求: pages={}, items={}",
            spider.name(),
            stop_ctx.pages,
            stop_ctx.items
        );
        return NextWorkResult::Continue;
    }

    let spider_name = spider.name().to_string();
    req.spider = Some(spider_name.clone());
    ctx.state
        .run
        .global_in_flight
        .fetch_add(1, Ordering::SeqCst);
    stats.in_flight.fetch_add(1, Ordering::SeqCst);
    ctx.state
        .run
        .in_flight_requests
        .lock()
        .await
        .entry(spider_name)
        .or_default()
        .push(req.clone());

    NextWorkResult::Work(Box::new(NextWork { spider, stats, req }))
}
