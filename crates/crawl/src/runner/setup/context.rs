//! EngineContext 构建。

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::auto;
use crate::engine;
use crate::runner::Engine;
use crate::scheduler;
use crate::{CrawlEvent, Request, Spider, SpiderStats};
use wisp_core::error::Result;
use wisp_fetcher::FetchClient;

impl Engine {
    pub(crate) fn build_rule_engine(&self) -> Result<Arc<Mutex<auto::ModeRuleEngine>>> {
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in &self.auto_rules {
            rule_engine.add_user_rule(pattern, *mode)?;
        }
        Ok(Arc::new(Mutex::new(rule_engine)))
    }

    fn build_engine_config(
        &self,
        fetch_client: &Arc<FetchClient>,
        fetch_mode: wisp_fetcher::FetchMode,
        max_concurrent: usize,
        obey_robots: bool,
    ) -> engine::EngineConfig {
        engine::EngineConfig {
            client: fetch_client.clone(),
            fetch_mode,
            max_concurrent,
            obey_robots,
            engine_max_pages: self.max_pages,
            max_refetch_rounds: self.max_refetch_rounds,
            max_retries: self.max_retries,
            checkpoint_store: self.checkpoint_store.clone(),
            checkpoint_interval: self.checkpoint_interval,
        }
    }

    fn build_engine_shared(
        &self,
        fetch_client: &Arc<FetchClient>,
        sched: Arc<scheduler::Scheduler>,
        follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
        follow_rx: tokio::sync::mpsc::UnboundedReceiver<Request>,
        rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
        robots_cache: Arc<crate::runtime::robots::RobotsCache>,
        fetch_mode: wisp_fetcher::FetchMode,
        obey_robots: bool,
    ) -> engine::EngineShared {
        engine::EngineShared {
            sched: sched.clone(),
            follow_tx,
            follow_rx: Arc::new(Mutex::new(follow_rx)),
            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            control: self.control.clone(),
            work_notify: Arc::new(tokio::sync::Notify::new()),
            event_bus: self.event_bus.clone(),
            middleware_chain: self.build_middleware_chain(
                fetch_client,
                &rule_engine,
                &robots_cache,
                fetch_mode,
                obey_robots,
            ),
            rule_engine,
            cf_domain_locks: Arc::new(dashmap::DashMap::new()),
        }
    }

    fn build_engine_state(
        &self,
        spiders: Vec<Arc<dyn Spider>>,
        tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
        items: Arc<Mutex<Vec<Value>>>,
        all_stats: Vec<Arc<SpiderStats>>,
    ) -> engine::EngineState {
        engine::EngineState {
            spiders,
            all_stats,
            items,
            abort_flag: Arc::new(AtomicBool::new(false)),
            tx,
            global_in_flight: Arc::new(AtomicUsize::new(0)),
            in_flight_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn build_engine_context(
        &self,
        spiders: Vec<Arc<dyn Spider>>,
        tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
        items: Arc<Mutex<Vec<Value>>>,
        sched: Arc<scheduler::Scheduler>,
        follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
        follow_rx: tokio::sync::mpsc::UnboundedReceiver<Request>,
        rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
        robots_cache: Arc<crate::runtime::robots::RobotsCache>,
        all_stats: Vec<Arc<SpiderStats>>,
    ) -> Arc<engine::EngineContext> {
        let fetch_client = self.fetch_client.clone();
        let fetch_mode = self.fetch_mode;
        let max_concurrent = self.max_concurrent;
        let obey_robots = self.obey_robots;
        Arc::new(engine::EngineContext {
            config: self.build_engine_config(
                &fetch_client,
                fetch_mode,
                max_concurrent,
                obey_robots,
            ),
            shared: self.build_engine_shared(
                &fetch_client,
                sched,
                follow_tx,
                follow_rx,
                rule_engine,
                robots_cache,
                fetch_mode,
                obey_robots,
            ),
            state: self.build_engine_state(spiders, tx, items, all_stats),
        })
    }
}
