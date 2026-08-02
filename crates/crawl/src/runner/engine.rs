//! Engine 基础设施主体。

use futures::stream::{self, StreamExt};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;

use super::config::EngineConfig;
use super::run_stream_driver;
use super::runtime::EngineRuntime;
use crate::control;
use crate::{CrawlEvent, CrawlStats, CrawlStream, Spider};
use wisp_core::error::Result;

/// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
///
/// 配置与运行时资源分离：`config` 是唯一配置源，`runtime` 持有可共享资源。
#[derive(Clone)]
pub struct Engine {
    /// 唯一配置源。
    pub(crate) config: EngineConfig,
    /// 运行时资源。
    pub(crate) runtime: EngineRuntime,
    /// 运行时并发保护。
    pub(crate) running: Arc<AtomicBool>,
}

impl Engine {
    /// 运行多个 Spider：共享队列 + callback 路由，每个 Spider 独立 until/stats。
    pub async fn run_many<S: Spider + 'static>(
        &self,
        spiders: Vec<S>,
    ) -> Result<(Vec<CrawlStats>, Vec<serde_json::Value>)> {
        let spiders: Vec<Arc<dyn Spider>> = spiders
            .into_iter()
            .map(|s| Arc::new(s) as Arc<dyn Spider>)
            .collect();
        let items: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let stats = self.run_inner_many(spiders, None, items.clone()).await?;
        let mut guard = items.lock().await;
        let item_list = std::mem::take(&mut *guard);
        Ok((stats, item_list))
    }

    /// 运行单个 Spider。返回 (统计, items)。
    pub async fn run<S: Spider + 'static>(
        &self,
        spider: S,
    ) -> Result<(CrawlStats, Vec<serde_json::Value>)> {
        let (mut stats, items) = self.run_many(vec![spider]).await?;
        Ok((stats.remove(0), items))
    }

    /// 流式运行：边爬边产出事件（仅单 Spider 模式）。
    pub fn run_stream<S: Spider + 'static>(&self, spider: S) -> CrawlStream {
        let stream = self.run_stream_many(vec![spider]);
        CrawlStream {
            inner: Box::pin(stream.inner.map(|event| match event {
                CrawlEvent::DoneMany(mut stats) => {
                    CrawlEvent::Done(stats.pop().unwrap_or_default())
                }
                other => other,
            })),
        }
    }

    /// 流式运行多个 Spider：共享队列 + callback 路由，每个 Spider 独立 until/stats。
    pub fn run_stream_many<S: Spider + 'static>(&self, spiders: Vec<S>) -> CrawlStream {
        let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
        let engine = self.clone();
        let spiders: Vec<Arc<dyn Spider>> = spiders
            .into_iter()
            .map(|s| Arc::new(s) as Arc<dyn Spider>)
            .collect();
        let driver = Box::pin(run_stream_driver(engine, spiders, tx.clone()));
        let rx = tokio_stream::wrappers::ReceiverStream::new(rx);
        let s = stream::unfold(
            (driver, rx, false),
            |(mut driver, mut rx, driver_done)| async move {
                if driver_done {
                    return rx.next().await.map(|e| (e, (driver, rx, true)));
                }
                tokio::select! {
                    biased;
                    event = rx.next() => event.map(|e| (e, (driver, rx, false))),
                    _ = &mut driver => {
                        rx.next().await.map(|e| (e, (driver, rx, true)))
                    }
                }
            },
        );
        CrawlStream { inner: Box::pin(s) }
    }

    /// 获取控制句柄（用于外部 pause/resume/cancel/shutdown）。
    pub fn control(&self) -> &Arc<control::EngineControl> {
        &self.runtime.control
    }

    /// 获取 Engine 配置。
    #[must_use]
    pub fn config(&self) -> EngineConfig {
        self.config.clone()
    }

    /// 关闭 Engine（停止所有运行中的爬取）。
    pub fn shutdown(&self) {
        self.runtime.control.shutdown();
    }
}
