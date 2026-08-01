//! EngineBuilder 构建逻辑。

use super::*;

impl EngineBuilder {
    /// 构建引擎实例。
    pub fn build(self) -> Result<Engine> {
        if self.max_concurrent == 0 {
            return Err(wisp_core::error::WispError::Config(
                "max_concurrent must be > 0".into(),
            ));
        }
        let fetch_client = match self.fetch_client {
            Some(client) => client,
            None => Arc::new(FetchClient::new(self.fetch_client_config)?),
        };
        Ok(Engine {
            fetch_client,
            cache_store: self.cache_store,
            max_concurrent: self.max_concurrent,
            max_pages: self.max_pages,
            max_refetch_rounds: self.max_refetch_rounds,
            checkpoint_store: self.checkpoint_store,
            checkpoint_interval: self.checkpoint_interval,
            control: Arc::new(control::EngineControl::new()),
            autoscale: self.autoscale,
            running: Arc::new(AtomicBool::new(false)),
            event_bus: Arc::new(self.event_bus),
            // 引擎配置（ND-031-ARCH）
            fetch_mode: self.fetch_mode,
            obey_robots: self.obey_robots,
            max_retries: self.max_retries,
            download_delay: self.download_delay,
            headers: self.headers,
            ua_middleware: self.ua_middleware,
            cookie_challenge: self.cookie_challenge,
            dynamic_upgrade: self.dynamic_upgrade,
            custom_middlewares: self.custom_middlewares,
            pipelines: self.pipelines,
            auto_rules: self.auto_rules,
        })
    }
}
