//! Engine lifecycle orchestration.

use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;

use super::Engine;
use super::{RunGuard, build_final_stats, run_work_loop};
use crate::engine;
use crate::robots;
use crate::scheduler;
use crate::stats::SpiderStats;
use crate::{CrawlEvent, CrawlStats, Request, Spider};
use wisp_core::error::Result;

impl Engine {
    async fn run_middleware_init(&self, ctx: &Arc<engine::EngineContext>) {
        if ctx.state.middleware_chain.is_empty() {
            return;
        }
        for (spider, stats) in ctx.state.spiders.iter().zip(&ctx.state.all_stats) {
            let crawl_ctx = engine::build_crawl_context_for(ctx, spider, stats);
            ctx.state.middleware_chain.run_init(&crawl_ctx).await;
            ctx.state
                .middleware_chain
                .run_pipelines_open(&crawl_ctx)
                .await;
        }
    }

    async fn run_middleware_close(&self, ctx: &Arc<engine::EngineContext>) {
        if ctx.state.middleware_chain.is_empty() {
            return;
        }
        for (spider, stats) in ctx.state.spiders.iter().zip(&ctx.state.all_stats) {
            let crawl_ctx = engine::build_crawl_context_for(ctx, spider, stats);
            ctx.state
                .middleware_chain
                .run_pipelines_close(&crawl_ctx)
                .await;
        }
    }

    fn spawn_autoscaler(
        &self,
        ctx: &Arc<engine::EngineContext>,
        all_stats: &[Arc<SpiderStats>],
    ) -> Option<tokio::task::JoinHandle<()>> {
        let pool = self.runtime.autoscale.clone()?;
        pool.set_work_notify(Arc::clone(&ctx.state.work_notify));
        let stats = all_stats.to_vec();
        Some(tokio::spawn(async move {
            pool.run_autoscaler(stats).await;
        }))
    }

    async fn cleanup_checkpoints(&self, ctx: &Arc<engine::EngineContext>) {
        let Some(store) = &self.runtime.checkpoint_store else {
            return;
        };
        for spider in &ctx.state.spiders {
            if let Err(e) = wisp_storage::delete_checkpoint(store.as_ref(), spider.name()).await {
                tracing::warn!("checkpoint 清理失败: spider={}, err={e}", spider.name());
            }
        }
    }

    async fn emit_finished_events(&self, final_stats: &[CrawlStats]) {
        for stats in final_stats {
            self.runtime
                .event_bus
                .emit(CrawlEvent::CrawlFinished {
                    stats: stats.clone(),
                })
                .await;
        }
    }

    /// 内部运行逻辑：共享队列驱动多个 Spider。
    async fn finish_run(&self, ctx: &Arc<engine::EngineContext>) -> Result<Vec<CrawlStats>> {
        self.run_middleware_close(ctx).await;
        for spider in &ctx.state.spiders {
            spider.on_close().await;
        }
        let interrupted =
            ctx.state.abort_flag.load(Ordering::SeqCst) || ctx.runtime.control.is_shutdown();
        if !interrupted {
            self.cleanup_checkpoints(ctx).await;
        }
        let final_stats = build_final_stats(ctx);
        self.emit_finished_events(&final_stats).await;
        Ok(final_stats)
    }

    /// 内部运行逻辑：共享队列驱动多个 Spider。
    /// 内部运行逻辑：共享队列驱动多个 Spider。
    pub(crate) async fn run_inner_many(
        &self,
        spiders: Vec<Arc<dyn Spider>>,
        items: Arc<Mutex<Vec<Value>>>,
    ) -> Result<Vec<CrawlStats>> {
        if spiders.is_empty() {
            return Ok(Vec::new());
        }
        let _guard = RunGuard::acquire(&self.running)?;
        self.runtime.control.reset().await;

        let all_stats: Vec<Arc<SpiderStats>> = spiders
            .iter()
            .map(|_| Arc::new(SpiderStats::new()))
            .collect();
        let rule_engine = self.build_rule_engine()?;
        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(robots::RobotsCache::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();

        self.restore_checkpoints(&spiders, &sched, &all_stats).await;
        self.seed_start_urls(&spiders, &sched).await;
        self.notify_spiders_start(&spiders).await;
        let ctx = self.build_engine_context(
            spiders,
            items,
            sched,
            follow_tx,
            follow_rx,
            rule_engine,
            robots_cache,
            all_stats.clone(),
        );

        self.run_middleware_init(&ctx).await;
        let autoscaler_handle = self.spawn_autoscaler(&ctx, &all_stats);
        run_work_loop(&ctx, self.runtime.autoscale.clone()).await;
        if let Some(handle) = autoscaler_handle {
            handle.abort();
        }
        self.finish_run(&ctx).await
    }
}
