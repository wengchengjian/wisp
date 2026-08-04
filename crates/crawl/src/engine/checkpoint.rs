//! Engine 子模块：checkpoint。

use super::*;
use std::collections::HashSet;

/// 到点则持久化 Spider checkpoint；未配置存储或未到 interval 时直接返回。
pub(crate) async fn maybe_persist_checkpoint(
    ctx: &EngineContext,
    spider: &Arc<dyn Spider>,
    stats: &Arc<SpiderStats>,
) {
    let Some(store) = &ctx.runtime.checkpoint_store else {
        return;
    };
    let interval = ctx.config.checkpoint_interval.max(1);
    let pages = stats.pages.load(Ordering::SeqCst);
    if pages == 0 || !pages.is_multiple_of(interval) {
        return;
    }
    let in_flight = ctx
        .state
        .run
        .in_flight_requests
        .lock()
        .await
        .get(spider.name())
        .cloned()
        .unwrap_or_default();
    let state = CrawlState {
        spider_name: spider.name().to_string(),
        pending_urls: ctx.state.queue.sched.pending_urls().await,
        seen_urls: ctx.state.queue.sched.seen_urls().await, // 持久化 seen 去重集合
        stats: stats.snapshot(),
        in_flight_urls: in_flight,
        saved_at: chrono::Utc::now(),
    };
    let blob = match bincode::serialize(&state) {
        Ok(blob) => blob,
        Err(e) => {
            tracing::warn!("checkpoint 序列化失败: {e}");
            return;
        }
    };
    if let Err(e) = store.save_checkpoint(spider.name(), &blob).await {
        tracing::warn!("checkpoint 保存失败: {e}");
        return;
    }
    ctx.runtime
        .event_bus
        .emit(CrawlEvent::CheckpointSaved {
            pending: ctx.state.queue.sched.len().await,
        })
        .await;
}

/// 加载并反序列化某个 Spider 的 checkpoint。
pub(crate) async fn load_spider_checkpoint(
    store: &dyn wisp_storage::Store,
    spider_name: &str,
) -> Result<Option<CrawlState>> {
    let Some(blob) = store.load_checkpoint(spider_name).await? else {
        return Ok(None);
    };
    bincode::deserialize(&blob).map(Some).map_err(|e| {
        wisp_core::error::WispError::Storage(wisp_core::error::StorageError::General(format!(
            "checkpoint 反序列化失败: {e}"
        )))
    })
}

/// 合并多个 Spider 的 checkpoint：pending/in-flight 按 URL 去重，seen 集合合并。
pub(crate) fn merge_checkpoint_states(
    states: Vec<CrawlState>,
) -> (Vec<CrawlRequest>, HashSet<String>) {
    let mut pending: Vec<CrawlRequest> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut inserted: HashSet<String> = HashSet::new();
    for state in states {
        seen.extend(state.seen_urls);
        for req in state.pending_urls {
            if inserted.insert(req.url.clone()) {
                pending.push(req);
            }
        }
        for req in state.in_flight_urls {
            if inserted.insert(req.url.clone()) {
                pending.push(req);
            }
        }
    }
    (pending, seen)
}

impl Engine {
    pub(crate) async fn restore_checkpoints(
        &self,
        spiders: &[Arc<dyn Spider>],
        sched: &Arc<scheduler::Scheduler>,
        all_stats: &[Arc<SpiderStats>],
    ) {
        let Some(store) = &self.runtime.checkpoint_store else {
            return;
        };
        let mut states: Vec<CrawlState> = Vec::new();
        for (i, spider) in spiders.iter().enumerate() {
            match load_spider_checkpoint(store.as_ref(), spider.name()).await {
                Ok(Some(state)) if state.spider_name == spider.name() => {
                    all_stats[i].restore_from(&state);
                    tracing::info!(
                        "checkpoint 恢复: spider={}, pending={}, seen={}, in_flight={}",
                        spider.name(),
                        state.pending_urls.len(),
                        state.seen_urls.len(),
                        state.in_flight_urls.len()
                    );
                    states.push(state);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("checkpoint 加载失败: spider={}, err={e}", spider.name())
                }
            }
        }
        let (pending, seen) = merge_checkpoint_states(states);
        if !pending.is_empty() || !seen.is_empty() {
            sched.import_state(pending, seen).await;
        }
    }
}
