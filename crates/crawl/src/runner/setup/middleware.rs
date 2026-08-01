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
                delay: self.download_delay,
                headers: self.headers.clone(),
                ua_middleware: self.ua_middleware.clone(),
                cookie_challenge: self.cookie_challenge,
                dynamic_upgrade: self.dynamic_upgrade,
                obey_robots,
                cache_store: self.cache_store.clone(),
                http_client: fetch_client.http_arc(),
                robots_cache: robots_cache.clone(),
                rule_engine: rule_engine.clone(),
            },
        );
        let mut chain = middleware::MiddlewareChain::new();
        chain.middlewares = self.custom_middlewares.clone();
        chain.middlewares.extend(defaults);
        chain.pipelines = self.pipelines.clone();
        chain.sort();
        Arc::new(chain)
    }
}
