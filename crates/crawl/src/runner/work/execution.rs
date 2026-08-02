//! 单个任务的执行。

use std::sync::Arc;

use super::NextWork;
use crate::engine;

impl NextWork {
    pub(super) async fn run(self, ctx: Arc<engine::EngineContext>) {
        let NextWork { spider, stats, req } = self;
        let spider_name = spider.name().to_string();
        let req_url = req.url.clone();
        let _g1 = engine::InFlightGuard {
            counter: ctx.state.global_in_flight.clone(),
            work_notify: Some(ctx.shared.work_notify.clone()),
        };
        let _g2 = engine::InFlightGuard {
            counter: stats.in_flight.clone(),
            work_notify: None,
        };
        if let Some(resp) = engine::process_request(&ctx, req).await {
            engine::process_response(&ctx, resp).await;
        }
        if let Some(v) = ctx
            .state
            .in_flight_requests
            .lock()
            .await
            .get_mut(&spider_name)
        {
            v.retain(|r| r.url != req_url);
        }
    }
}
