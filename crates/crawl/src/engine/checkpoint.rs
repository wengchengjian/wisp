//! Engine 子模块：checkpoint。

use super::*;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// 到点则持久化 Spider checkpoint；未配置存储或未到 interval 时直接返回。
///
/// 触发条件（任一满足）：
/// - 页数间隔：`stats.pages` 为 `checkpoint_interval` 的倍数（且 > 0）
/// - 时间间隔：配置了 `checkpoint_interval_secs`，且距上次保存已超过该秒数
///
/// 触发判断在主路径同步完成（廉价）；快照/序列化/写盘 spawn 到后台任务，
/// 避免全量 pending 排序 + 文件 IO 阻塞响应处理主循环（大数据量队列时此
/// 路径会把 16 并发拖垮到 ~1 页/秒）。
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

    // 时间间隔判断：自上次保存超过 interval_secs 也触发一次（慢速爬取兜底）
    let time_due = match ctx.config.checkpoint_interval_secs {
        Some(secs) if secs > 0 => {
            let last = ctx.runtime.last_checkpoint_at.lock().await;
            match *last {
                Some(prev) => prev.elapsed() >= Duration::from_secs(secs),
                None => true, // 从未保存过：首次到达即保存
            }
        }
        _ => false,
    };
    if pages == 0 || (!pages.is_multiple_of(interval) && !time_due) {
        return;
    }
    // 防重入：已有后台保存在进行则跳过本次（下一触发点会再保存）
    if ctx.runtime.checkpoint_saving.swap(true, Ordering::SeqCst) {
        return;
    }

    // clone 快照所需的所有 Arc 引用（均 'static，可安全 spawn）
    let store = Arc::clone(store);
    let sched = Arc::clone(&ctx.state.queue.sched);
    let in_flight_map = Arc::clone(&ctx.state.run.in_flight_requests);
    let last_at = Arc::clone(&ctx.runtime.last_checkpoint_at);
    let event_bus = Arc::clone(&ctx.runtime.event_bus);
    let saving_flag = Arc::clone(&ctx.runtime.checkpoint_saving);
    let spider = Arc::clone(spider);
    let stats = Arc::clone(stats);

    tokio::spawn(async move {
        let started = Instant::now();
        let in_flight = in_flight_map
            .lock()
            .await
            .get(spider.name())
            .cloned()
            .unwrap_or_default();
        let state = CrawlState {
            spider_name: spider.name().to_string(),
            pending_urls: sched.pending_urls().await,
            seen_urls: sched.seen_urls().await, // 持久化 seen 去重集合
            stats: stats.snapshot(),
            in_flight_urls: in_flight,
            saved_at: chrono::Utc::now(),
        };
        let blob = match bincode::serialize(&state) {
            Ok(blob) => blob,
            Err(e) => {
                tracing::warn!("checkpoint 序列化失败: {e}");
                saving_flag.store(false, Ordering::SeqCst);
                return;
            }
        };
        if let Err(e) = store.save_checkpoint(spider.name(), &blob).await {
            tracing::warn!("checkpoint 保存失败: {e}");
            saving_flag.store(false, Ordering::SeqCst);
            return;
        }
        // 更新最近保存时间（无论按页数还是时间触发）
        *last_at.lock().await = Some(Instant::now());
        saving_flag.store(false, Ordering::SeqCst);
        event_bus
            .emit(CrawlEvent::CheckpointSaved {
                pending: sched.len().await,
            })
            .await;
        tracing::info!(
            "checkpoint 保存完成: spider={}, pending={}, blob={}KB, 耗时={:?}",
            spider.name(),
            state.pending_urls.len(),
            blob.len() / 1024,
            started.elapsed()
        );
    });
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
