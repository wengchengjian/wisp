//! EngineBuilder 构建逻辑。

use super::*;
use crate::control;
use crate::engine::runtime::EngineRuntime;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use wisp_core::error::Result;
use wisp_fetcher::FetchClient;

impl EngineBuilder {
    /// 构建引擎实例。
    pub fn build(mut self) -> Result<Engine> {
        if self.config.max_concurrent == 0 {
            return Err(wisp_core::error::WispError::Config(
                "max_concurrent must be > 0".into(),
            ));
        }
        let fetch_client = match self.fetch_client {
            Some(client) => {
                self.config.transport = client.config().clone();
                client
            }
            None => Arc::new(FetchClient::new(self.config.transport.clone())?),
        };
        let runtime = EngineRuntime {
            fetch_client,
            control: Arc::new(control::EngineControl::new()),
            cache_store: self.cache_store,
            checkpoint_store: self.checkpoint_store,
            autoscale: self.autoscale,
            event_bus: Arc::new(self.event_bus),
            ua_middleware: self.ua_middleware,
            custom_middlewares: self.custom_middlewares,
            pipelines: self.pipelines,
        };
        Ok(Engine {
            config: self.config,
            runtime,
            running: Arc::new(AtomicBool::new(false)),
        })
    }
}
