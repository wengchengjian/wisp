//! 调度决策与任务选取。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{NextWork, NextWorkResult};
use crate::engine;
use crate::stop;
use crate::{Spider, SpiderStats};
use wisp_core::utils::sanitize_url;

async fn drain_follow_queue(ctx: &engine::EngineContext) {
    let mut rx_guard = ctx.state.follow_rx.lock().await;
    while let Ok(req) = rx_guard.try_recv() {
        ctx.state.sched.push(req).await;
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
    if ctx.state.abort_flag.load(Ordering::SeqCst) {
        return Some(NextWorkResult::Done);
    }
    if ctx.runtime.control.is_shutdown() {
        return Some(done_or_wait(
            ctx.state.global_in_flight.load(Ordering::SeqCst),
        ));
    }
    drain_follow_queue(ctx).await;
    let total_pages: usize = ctx
        .state
        .all_stats
        .iter()
        .map(|s| s.pages.load(Ordering::SeqCst))
        .sum();
    if total_pages + ctx.state.global_in_flight.load(Ordering::SeqCst) >= ctx.config.max_pages {
        return Some(done_or_wait(
            ctx.state.global_in_flight.load(Ordering::SeqCst),
        ));
    }
    let limit = match autoscale {
        Some(pool) => pool.current_concurrency(),
        None => ctx.config.max_concurrent,
    };
    if ctx.state.global_in_flight.load(Ordering::SeqCst) >= limit {
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
    let queue_size = ctx.state.sched.len().await;
    let mut req = match ctx.state.sched.pop().await {
        Some(req) => req,
        None => return done_or_wait(ctx.state.global_in_flight.load(Ordering::SeqCst)),
    };
    let Some(idx) = ctx.state.spider_index_for(&req) else {
        tracing::warn!("丢弃无 Spider 接收的请求: url={}", sanitize_url(&req.url));
        return NextWorkResult::Continue;
    };
    let spider = Arc::clone(&ctx.state.spiders[idx]);
    let stats = Arc::clone(&ctx.state.all_stats[idx]);
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
    ctx.state.global_in_flight.fetch_add(1, Ordering::SeqCst);
    stats.in_flight.fetch_add(1, Ordering::SeqCst);
    ctx.state
        .in_flight_requests
        .lock()
        .await
        .entry(spider_name)
        .or_default()
        .push(req.clone());

    NextWorkResult::Work(Box::new(NextWork { spider, stats, req }))
}
