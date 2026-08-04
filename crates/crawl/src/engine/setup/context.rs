//! EngineContext 构建。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::sync::Mutex;

use crate::auto;
use crate::engine;
use crate::engine::Engine;
use wisp_core::error::Result;

impl Engine {
    pub(crate) fn build_rule_engine(&self) -> Result<Arc<Mutex<auto::ModeRuleEngine>>> {
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in &self.config.auto_rules {
            rule_engine.add_user_rule(pattern, *mode)?;
        }
        Ok(Arc::new(Mutex::new(rule_engine)))
    }

    pub(crate) fn build_engine_context(
        &self,
        draft: engine::EngineRunDraft,
    ) -> Arc<engine::EngineContext> {
        let middleware_chain = {
            let defaults = crate::middleware::builtin::default_middlewares(
                crate::middleware::builtin::DefaultMiddlewareConfig {
                    fetch_mode: self.config.fetch_mode,
                    delay: self.config.download_delay,
                    headers: self.config.headers.clone(),
                    ua_middleware: self.runtime.ua_middleware.clone(),
                    cookie_challenge: self.config.cookie_challenge,
                    dynamic_upgrade: self.config.dynamic_upgrade,
                    obey_robots: self.config.obey_robots,
                    cache_store: self.runtime.cache_store.clone(),
                    http_client: self.runtime.fetch_client.clone(),
                    robots_cache: draft.robots_cache.clone(),
                    rule_engine: draft.rule_engine.clone(),
                },
            );
            let mut chain = crate::middleware::MiddlewareChain::new();
            chain.middlewares = self.runtime.custom_middlewares.clone();
            chain.middlewares.extend(defaults);
            chain.pipelines = self.runtime.pipelines.clone();
            chain.sort();
            Arc::new(chain)
        };
        Arc::new(engine::EngineContext {
            config: self.config.clone(),
            runtime: self.runtime.clone(),
            state: engine::EngineState {
                queue: engine::QueueState {
                    sched: draft.sched.clone(),
                    follow_tx: draft.follow_tx,
                    follow_rx: Arc::new(Mutex::new(draft.follow_rx)),
                    work_notify: Arc::new(tokio::sync::Notify::new()),
                },
                middleware_chain,
                rule_engine: draft.rule_engine,
                cf_locks: engine::CfLockMap {
                    locks: Arc::new(dashmap::DashMap::new()),
                },
                spiders: engine::SpiderRegistry {
                    spiders: draft.spiders,
                    all_stats: draft.all_stats,
                },
                run: engine::RunState {
                    abort_flag: Arc::new(AtomicBool::new(false)),
                    pipeline_error: Arc::new(Mutex::new(None)),
                    global_in_flight: Arc::new(AtomicUsize::new(0)),
                    in_flight_requests: Arc::new(Mutex::new(HashMap::new())),
                },
            },
        })
    }
}
