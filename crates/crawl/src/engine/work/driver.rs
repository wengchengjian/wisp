//! 流式运行驱动。

use std::sync::Arc;

use super::Engine;
use crate::{CrawlEvent, CrawlStats, Spider};
use wisp_core::error::Result;

pub(crate) async fn run_stream_driver(
    engine: Engine,
    spiders: Vec<Arc<dyn Spider>>,
    tx: tokio::sync::mpsc::Sender<CrawlEvent>,
    outcome: tokio::sync::oneshot::Sender<Result<Vec<CrawlStats>>>,
) {
    match engine.run_inner_many(spiders).await {
        Ok(stats) => {
            let _ = tx.send(CrawlEvent::DoneMany(stats.clone())).await;
            let _ = outcome.send(Ok(stats));
        }
        Err(e) => {
            let _ = tx
                .send(CrawlEvent::Error {
                    url: "*".into(),
                    error: e.to_string(),
                    attempt: 0,
                })
                .await;
            let _ = tx.send(CrawlEvent::DoneMany(Vec::new())).await;
            let _ = outcome.send(Err(e));
        }
    }
}
