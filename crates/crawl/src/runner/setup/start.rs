//! 起始 URL 播种与 Spider 生命周期通知。

use std::sync::Arc;

use crate::observability::events::EngineEvent;
use crate::runner::Engine;
use crate::scheduler;
use crate::{Request, Spider};
use wisp_core::utils::sanitize_url;

impl Engine {
    pub(crate) async fn seed_start_urls(
        &self,
        spiders: &[Arc<dyn Spider>],
        sched: &Arc<scheduler::Scheduler>,
    ) {
        for spider in spiders {
            let start_urls = spider.start_urls();
            self.event_bus
                .emit(EngineEvent::CrawlStarted {
                    spider: spider.name().to_string(),
                    start_urls: start_urls.len(),
                })
                .await;
            for url in start_urls {
                sched
                    .push(Request::get(&url).with_spider(spider.name()))
                    .await;
                self.event_bus
                    .emit(EngineEvent::RequestScheduled {
                        url: sanitize_url(&url),
                        depth: 0,
                    })
                    .await;
            }
        }
    }

    pub(crate) async fn notify_spiders_start(&self, spiders: &[Arc<dyn Spider>]) {
        for spider in spiders {
            spider.on_start().await;
        }
    }
}
