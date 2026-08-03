//! 流式运行驱动。

use std::sync::Arc;
use tokio::sync::Mutex;

use super::Engine;
use crate::{CrawlEvent, Spider};

pub(crate) async fn run_stream_driver(
    engine: Engine,
    spiders: Vec<Arc<dyn Spider>>,
    tx: tokio::sync::mpsc::Sender<CrawlEvent>,
) {
    let items = Arc::new(Mutex::new(Vec::new()));
    match engine
        .run_inner_many(spiders, Some(tx.clone()), items)
        .await
    {
        Ok(stats) => {
            let _ = tx.send(CrawlEvent::DoneMany(stats)).await;
        }
        Err(e) => {
            let _ = tx
                .send(CrawlEvent::Error {
                    url: "*".into(),
                    error: e.to_string(),
                })
                .await;
            let _ = tx.send(CrawlEvent::DoneMany(Vec::new())).await;
        }
    }
}
