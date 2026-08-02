//! 中间件链构建。

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::auto;
use crate::middleware;
use crate::runner::Engine;
use wisp_fetcher::FetchClient;

impl Engine {
    pub(super) fn build_middleware_chain(
        &self,
        fetch_client: &Arc<FetchClient>,
        rule_engine: &Arc<Mutex<auto::ModeRuleEngine>>,
        robots_cache: &Arc<crate::runtime::robots::RobotsCache>,
        fetch_mode: wisp_fetcher::FetchMode,
        obey_robots: bool,
    ) -> Arc<middleware::MiddlewareChain> {
        let defaults = middleware::builtin::default_middlewares(
            middleware::builtin::DefaultMiddlewareConfig {
                fetch_mode,
                delay: self.config.download_delay,
                headers: self.config.headers.clone(),
                ua_middleware: self.runtime.ua_middleware.clone(),
                cookie_challenge: self.config.cookie_challenge,
                dynamic_upgrade: self.config.dynamic_upgrade,
                obey_robots,
                cache_store: self.runtime.cache_store.clone(),
                http_client: fetch_client.clone(),
                robots_cache: robots_cache.clone(),
                rule_engine: rule_engine.clone(),
            },
        );
        let mut chain = middleware::MiddlewareChain::new();
        chain.middlewares = self.runtime.custom_middlewares.clone();
        chain.middlewares.extend(defaults);
        chain.pipelines = self.runtime.pipelines.clone();
        chain.sort();
        Arc::new(chain)
    }
}
