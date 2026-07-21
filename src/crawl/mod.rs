//! Spider-based crawling engine.

pub mod scheduler;
pub mod robots;
pub mod cache;
pub mod templates;

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;

use crate::error::{WispError, Result};
use crate::fetch::{self, Client};
use crate::parser::Node;

/// HTTP method for spider requests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Method { Get, Post, Put, Delete }

/// A request to be processed by the spider.
#[derive(Debug, Clone)]
pub struct SpiderRequest {
    pub url: String,
    pub method: Method,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub meta: Value,
    pub callback: Option<String>,
    pub priority: i32,
}

impl SpiderRequest {
    pub fn get(url: &str) -> Self {
        Self { url: url.to_string(), method: Method::Get, headers: HashMap::new(), body: None, meta: Value::Null, callback: None, priority: 0 }
    }
    pub fn post(url: &str, body: Option<String>) -> Self {
        Self { url: url.to_string(), method: Method::Post, headers: HashMap::new(), body, meta: Value::Null, callback: None, priority: 0 }
    }
    pub fn with_meta(mut self, meta: Value) -> Self { self.meta = meta; self }
    pub fn with_priority(mut self, p: i32) -> Self { self.priority = p; self }
    pub fn with_callback(mut self, cb: &str) -> Self { self.callback = Some(cb.to_string()); self }
}

/// Response received by the spider.
#[derive(Debug, Clone)]
pub struct SpiderResponse {
    pub url: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub request: SpiderRequest,
}

impl SpiderResponse {
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone())
            .map_err(|e| WispError::CdpError(format!("utf8 decode: {e}")))
    }
    pub fn parse(&self) -> Result<Node> {
        let text = self.text()?;
        Ok(Node::from_html(&text))
    }
    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.body)
            .map_err(|e| WispError::CdpError(format!("json: {e}")))
    }
}

/// The core Spider trait users implement to define a crawler.
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    // Required
    fn name(&self) -> &str;
    fn start_urls(&self) -> Vec<String>;
    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>);

    // Optional with defaults
    fn allowed_domains(&self) -> HashSet<String> { HashSet::new() }
    fn concurrent_requests(&self) -> u32 { 8 }
    fn download_delay(&self) -> Duration { Duration::from_millis(0) }
    fn obey_robots(&self) -> bool { true }
    fn max_retries(&self) -> u32 { 3 }
    fn fetcher_config(&self) -> fetch::Config { fetch::Config::default() }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
    async fn on_error(&self, _req: &SpiderRequest, _err: &str) {}
    async fn on_item(&self, item: Value) -> Option<Value> { Some(item) }
    fn is_blocked(&self, _resp: &SpiderResponse) -> bool { false }
}

/// Crawling statistics.
#[derive(Debug, Clone)]
pub struct CrawlStats {
    pub items_scraped: usize,
    pub pages_crawled: usize,
    pub errors: usize,
    pub duration: Duration,
}

/// Engine configuration.
pub struct EngineConfig {
    pub max_pages: usize,
    pub max_concurrent: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { max_pages: 1000, max_concurrent: 8 }
    }
}

/// The crawling engine that drives a Spider.
pub struct Engine<S: Spider> {
    spider: S,
    config: EngineConfig,
}

impl<S: Spider> Engine<S> {
    pub fn new(spider: S) -> Self {
        let max_concurrent = spider.concurrent_requests() as usize;
        Self {
            spider,
            config: EngineConfig {
                max_concurrent,
                ..Default::default()
            },
        }
    }

    pub fn max_pages(mut self, n: usize) -> Self { self.config.max_pages = n; self }
    pub fn max_concurrent(mut self, n: usize) -> Self { self.config.max_concurrent = n; self }

    pub async fn run(self) -> Result<CrawlStats> {
        let start = std::time::Instant::now();
        // 提前提取所有需要的信息（避免 self 部分移动问题）
        let max_pages = self.config.max_pages;
        let max_concurrent = self.config.max_concurrent;
        let obey_robots = self.spider.obey_robots();
        let allowed = self.spider.allowed_domains();
        let start_urls = self.spider.start_urls();
        let fetcher_config = self.spider.fetcher_config();

        let client = Client::builder()
            .timeout(fetcher_config.timeout)
            .build()?;

        self.spider.on_start().await;

        let spider = Arc::new(self.spider);
        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(Mutex::new(robots::RobotsCache::new()));

        // Seed start URLs
        for url in start_urls {
            sched.push(SpiderRequest::get(&url)).await;
        }

        // Channel for follow requests 回灌
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();
        let stats_items = Arc::new(AtomicUsize::new(0));
        let stats_pages = Arc::new(AtomicUsize::new(0));
        let stats_errors = Arc::new(AtomicUsize::new(0));

        // Domain semaphores for per-domain throttling
        let domain_sems: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let follow_rx = Arc::new(Mutex::new(follow_rx));
        let client = Arc::new(client);
        let allowed = Arc::new(allowed);

        let stream = {
            let sched = sched.clone();
            let follow_rx = follow_rx.clone();
            let follow_tx = follow_tx.clone();
            let spider = spider.clone();
            let client = client.clone();
            let stats_pages = stats_pages.clone();
            let stats_errors = stats_errors.clone();
            let stats_items = stats_items.clone();
            let domain_sems = domain_sems.clone();
            let robots_cache = robots_cache.clone();
            let allowed = allowed.clone();

            stream::unfold((), move |_| {
                let sched = sched.clone();
                let follow_rx = follow_rx.clone();
                let follow_tx = follow_tx.clone();
                let spider = spider.clone();
                let client = client.clone();
                let stats_pages = stats_pages.clone();
                let stats_errors = stats_errors.clone();
                let stats_items = stats_items.clone();
                let domain_sems = domain_sems.clone();
                let robots_cache = robots_cache.clone();
                let allowed = allowed.clone();

                async move {
                    // 1. Drain follow channel into scheduler
                    let mut rx_guard = follow_rx.lock().await;
                    while let Ok(req) = rx_guard.try_recv() {
                        sched.push(req).await;
                    }
                    drop(rx_guard);

                    // 2. Check page budget
                    if stats_pages.load(Ordering::SeqCst) >= max_pages {
                        return None;
                    }

                    // 3. Pop next request
                    let req = sched.pop().await?;

                    // 4-7. All logic in a single async block (unified future type)
                    let spider_clone = spider.clone();
                    let stats_pages_c = stats_pages.clone();
                    let stats_errors_c = stats_errors.clone();
                    let stats_items_c = stats_items.clone();
                    let follow_tx_c = follow_tx.clone();
                    let client_c = client.clone();
                    let domain_sems_c = domain_sems.clone();
                    let robots_cache_c = robots_cache.clone();
                    let allowed_c = allowed.clone();

                    let fut = async move {
                        // 4. Domain filter
                        if !allowed_c.is_empty() {
                            if let Ok(parsed) = url::Url::parse(&req.url) {
                                if let Some(host) = parsed.host_str() {
                                    if !allowed_c.contains(host) {
                                        return;  // skip
                                    }
                                }
                            }
                        }

                        // 5. Robots check
                        if obey_robots {
                            let url_clone = req.url.clone();
                            let client_r = client_c.clone();
                            let allowed = {
                                let mut rc = robots_cache_c.lock().await;
                                rc.is_allowed(&client_r, &url_clone).await
                            };
                            if !allowed {
                                return;
                            }
                        }

                        // 6. Per-domain throttle
                        let domain = url::Url::parse(&req.url)
                            .ok()
                            .and_then(|u| u.host_str().map(|s| s.to_string()))
                            .unwrap_or_default();
                        let sem = {
                            let mut sems = domain_sems_c.lock().await;
                            sems.entry(domain.clone())
                                .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
                                .clone()
                        };
                        let _permit = sem.acquire_owned().await.unwrap();

                        // 7. Fetch
                        match fetch_page(&client_c, &req).await {
                            Ok(resp) => {
                                if spider_clone.is_blocked(&resp) {
                                    stats_errors_c.fetch_add(1, Ordering::SeqCst);
                                    return;
                                }
                                stats_pages_c.fetch_add(1, Ordering::SeqCst);
                                let (items, follows) = spider_clone.parse(resp).await;
                                for item in items {
                                    if let Some(_processed) = spider_clone.on_item(item).await {
                                        stats_items_c.fetch_add(1, Ordering::SeqCst);
                                    }
                                }
                                for f in follows {
                                    let _ = follow_tx_c.send(f);
                                }
                            }
                            Err(e) => {
                                stats_errors_c.fetch_add(1, Ordering::SeqCst);
                                spider_clone.on_error(&req, &e.to_string()).await;
                            }
                        }
                    };

                    // Return the future for buffer_unordered
                    Some((fut, ()))
                }
            })
            .buffer_unordered(max_concurrent)
        };

        // Drive the stream to completion
        tokio::pin!(stream);
        while stream.next().await.is_some() {}

        spider.on_close().await;

        Ok(CrawlStats {
            items_scraped: stats_items.load(Ordering::SeqCst),
            pages_crawled: stats_pages.load(Ordering::SeqCst),
            errors: stats_errors.load(Ordering::SeqCst),
            duration: start.elapsed(),
        })
    }
}

async fn fetch_page(client: &Client, req: &SpiderRequest) -> Result<SpiderResponse> {
    let resp = match req.method {
        Method::Get => client.get(&req.url).await?,
        Method::Post => client.post(&req.url, req.body.as_deref(), None).await?,
        Method::Put => client.put(&req.url, req.body.as_deref(), None).await?,
        Method::Delete => client.delete(&req.url).await?,
    };

    Ok(SpiderResponse {
        url: resp.url.clone(),
        status: resp.status,
        headers: resp.headers.clone(),
        body: resp.body.clone(),
        request: req.clone(),
    })
}
