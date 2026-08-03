//! Checkpoint 恢复。

use std::collections::HashSet;
use std::sync::Arc;

use crate::engine;
use crate::engine::Engine;
use crate::scheduler;
use crate::{CrawlState, Request, Spider, SpiderStats};

/// 合并多个 Spider 的 checkpoint：pending/in-flight 按 URL 去重，seen 集合合并。
pub(crate) fn merge_checkpoint_states(states: Vec<CrawlState>) -> (Vec<Request>, HashSet<String>) {
    let mut pending: Vec<Request> = Vec::new();
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
            match engine::load_spider_checkpoint(store.as_ref(), spider.name()).await {
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
