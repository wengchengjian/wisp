//! EngineBuilder 构建逻辑。

use super::{Engine, EngineBuilder};
use crate::control;
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
        let fetch_client = match self.draft.fetch_client {
            Some(client) => {
                self.config.transport = client.config().clone();
                client
            }
            None => Arc::new(FetchClient::new(self.config.transport.clone())?),
        };
        self.draft.fetch_client = Some(fetch_client);
        let runtime = self
            .draft
            .into_runtime(Arc::new(control::EngineControl::new()));
        Ok(Engine {
            config: self.config,
            runtime,
            running: Arc::new(AtomicBool::new(false)),
        })
    }
}
