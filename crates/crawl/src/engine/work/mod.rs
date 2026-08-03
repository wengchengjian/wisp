//! Engine 运行工作循环与流驱动子模块。

mod driver;
mod execution;
mod scheduling;

use futures::stream::{self, StreamExt};
use std::sync::Arc;

use super::Engine;
use crate::engine;
use crate::{CrawlStats, Request, Spider, SpiderStats};

pub(crate) use driver::run_stream_driver;

struct NextWork {
    spider: Arc<dyn Spider>,
    stats: Arc<SpiderStats>,
    req: Request,
}

enum NextWorkResult {
    Work(Box<NextWork>),
    Wait,
    Continue,
    Done,
}

pub(crate) async fn run_work_loop(
    ctx: &Arc<engine::EngineContext>,
    autoscale: Option<Arc<crate::runtime::autoscale::AutoscaledPool>>,
) {
    let buffer_ceiling = autoscale
        .as_ref()
        .map_or(ctx.config.max_concurrent, |pool| pool.max_concurrency());
    let stream = {
        let ctx = ctx.clone();
        let autoscale = autoscale.clone();
        stream::unfold((), move |_| {
            let ctx = ctx.clone();
            let autoscale = autoscale.clone();
            async move {
                loop {
                    match scheduling::next_work(&ctx, autoscale.as_deref()).await {
                        NextWorkResult::Work(work) => {
                            let ctx = ctx.clone();
                            return Some(((*work).run(ctx.clone()), ()));
                        }
                        NextWorkResult::Wait => ctx.state.work_notify.notified().await,
                        NextWorkResult::Continue => continue,
                        NextWorkResult::Done => return None,
                    }
                }
            }
        })
    }
    .buffer_unordered(buffer_ceiling);
    tokio::pin!(stream);
    while stream.next().await.is_some() {}
}

pub(crate) fn build_final_stats(ctx: &Arc<engine::EngineContext>) -> Vec<CrawlStats> {
    ctx.state
        .all_stats
        .iter()
        .map(|stats| {
            let status_codes = stats.status_codes_snapshot();
            engine::snapshot_stats_for(stats, status_codes)
        })
        .collect()
}
