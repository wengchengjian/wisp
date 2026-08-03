//! 起始 URL 播种与 Spider 生命周期通知。

use std::sync::Arc;

use crate::engine::Engine;
use crate::scheduler;
use crate::{CrawlEvent, Request, Spider};

impl Engine {
    pub(crate) async fn seed_start_urls(
        &self,
        spiders: &[Arc<dyn Spider>],
        sched: &Arc<scheduler::Scheduler>,
    ) {
        for spider in spiders {
            let start_urls = spider.start_urls();
            self.runtime
                .event_bus
                .emit(CrawlEvent::CrawlStarted {
                    spider: spider.name().to_string(),
                    start_urls: start_urls.len(),
                })
                .await;
            for url in start_urls {
                sched
                    .push(Request::get(&url).with_spider(spider.name()))
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
