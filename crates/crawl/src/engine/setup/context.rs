//! EngineContext 构建。

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::sync::Mutex;

use crate::auto;
use crate::engine;
use crate::engine::Engine;
use crate::scheduler;
use crate::stats::SpiderStats;
use crate::{CrawlEvent, Request, Spider};
use wisp_core::error::Result;

impl Engine {
    pub(crate) fn build_rule_engine(&self) -> Result<Arc<Mutex<auto::ModeRuleEngine>>> {
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in &self.config.auto_rules {
            rule_engine.add_user_rule(pattern, *mode)?;
        }
        Ok(Arc::new(Mutex::new(rule_engine)))
    }

    // 内部构建参数较多，拆结构体会扩大 Engine 内部配置面。
    #[expect(clippy::too_many_arguments)]
    fn build_engine_state(
        &self,
        sched: Arc<scheduler::Scheduler>,
        follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
        follow_rx: tokio::sync::mpsc::UnboundedReceiver<Request>,
        rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
        robots_cache: Arc<crate::runtime::robots::RobotsCache>,
        spiders: Vec<Arc<dyn Spider>>,
        tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
        items: Arc<Mutex<Vec<Value>>>,
        all_stats: Vec<Arc<SpiderStats>>,
    ) -> engine::EngineState {
        let fetch_client = self.runtime.fetch_client.clone();
        engine::EngineState {
            sched: sched.clone(),
            follow_tx,
            follow_rx: Arc::new(Mutex::new(follow_rx)),
            work_notify: Arc::new(tokio::sync::Notify::new()),
            middleware_chain: self.build_middleware_chain(
                &fetch_client,
                &rule_engine,
                &robots_cache,
                self.config.fetch_mode,
                self.config.obey_robots,
            ),
            rule_engine,
            cf_domain_locks: Arc::new(dashmap::DashMap::new()),
            spiders,
            all_stats,
            items,
            abort_flag: Arc::new(AtomicBool::new(false)),
            tx,
            global_in_flight: Arc::new(AtomicUsize::new(0)),
            in_flight_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // 内部构建参数较多，拆结构体会扩大 Engine 内部配置面。
    #[expect(clippy::too_many_arguments)]
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
        Arc::new(engine::EngineContext {
            config: self.config.clone(),
            runtime: self.runtime.clone(),
            state: self.build_engine_state(
                sched,
                follow_tx,
                follow_rx,
                rule_engine,
                robots_cache,
                spiders,
                tx,
                items,
                all_stats,
            ),
        })
    }
}
