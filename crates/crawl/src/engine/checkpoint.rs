//! Engine 子模块：checkpoint。

use super::*;

pub(crate) async fn persist_spider_checkpoint(
    store: &dyn wisp_storage::Store,
    spider_name: &str,
    sched: &scheduler::Scheduler,
    stats: &Arc<SpiderStats>,
    in_flight: Vec<Request>,
) -> Result<()> {
    let pending = sched.pending_urls().await;
    let seen = sched.seen_urls().await; // 持久化 seen 去重集合
    let snapshot = snapshot_stats_for(stats, HashMap::new());
    // 手动构造 CrawlState 填入 seen_urls；
    // `CrawlState::from_stats` 硬编码 seen_urls 为空，不能直接用。
    let state = CrawlState {
        spider_name: spider_name.to_string(),
        pending_urls: pending,
        seen_urls: seen,
        items_scraped: snapshot.items_scraped,
        pages_crawled: snapshot.pages_crawled,
        errors: snapshot.errors,
        callback_pages: stats.callback_pages_snapshot(),
        in_flight_urls: in_flight,
        duration_ms: snapshot.duration.as_millis(),
        saved_at: chrono::Utc::now(),
    };
    let blob = bincode::serialize(&state).map_err(|e| {
        wisp_core::error::WispError::Storage(wisp_core::error::StorageError::General(format!(
            "checkpoint 序列化失败: {e}"
        )))
    })?;
    wisp_storage::save_checkpoint(store, spider_name, &blob)
        .await
        .map_err(|e| {
            wisp_core::error::WispError::Storage(wisp_core::error::StorageError::General(format!(
                "checkpoint 保存失败: {e}"
            )))
        })?;
    Ok(())
}

/// 加载并反序列化某个 Spider 的 checkpoint。
pub(crate) async fn load_spider_checkpoint(
    store: &dyn wisp_storage::Store,
    spider_name: &str,
) -> Result<Option<CrawlState>> {
    let Some(blob) = wisp_storage::load_checkpoint(store, spider_name).await? else {
        return Ok(None);
    };
    bincode::deserialize(&blob).map(Some).map_err(|e| {
        wisp_core::error::WispError::Storage(wisp_core::error::StorageError::General(format!(
            "checkpoint 反序列化失败: {e}"
        )))
    })
}
