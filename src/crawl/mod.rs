//! Spider-based crawling engine.

pub mod scheduler;
pub mod robots;
pub mod cache;
pub mod templates;

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use async_trait::async_trait;
use serde_json::Value;

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

/// The crawling engine that drives a Spider.
pub struct Engine<S: Spider> {
    spider: S,
    max_pages: usize,
}

impl<S: Spider> Engine<S> {
    pub fn new(spider: S) -> Self { Self { spider, max_pages: 1000 } }
    pub fn max_pages(mut self, n: usize) -> Self { self.max_pages = n; self }

    pub async fn run(self) -> Result<CrawlStats> {
        let start = std::time::Instant::now();
        let client = Client::builder()
            .timeout(self.spider.fetcher_config().timeout)
            .build()?;

        self.spider.on_start().await;

        let mut sched = scheduler::Scheduler::new();
        let mut robots_cache = robots::RobotsCache::new();
        let allowed = self.spider.allowed_domains();

        // Seed start URLs
        for url in self.spider.start_urls() {
            sched.push(SpiderRequest::get(&url));
        }

        let mut items_scraped = 0usize;
        let mut pages_crawled = 0usize;
        let mut errors = 0usize;

        while let Some(req) = sched.pop() {
            if pages_crawled >= self.max_pages { break; }

            // Domain filter
            if !allowed.is_empty() {
                if let Ok(parsed) = url::Url::parse(&req.url) {
                    if let Some(host) = parsed.host_str() {
                        if !allowed.contains(host) { continue; }
                    }
                }
            }

            // Robots check
            if self.spider.obey_robots() {
                if !robots_cache.is_allowed(&client, &req.url).await {
                    continue;
                }
            }

            // Fetch
            let resp = match self.fetch_page(&client, &req).await {
                Ok(r) => r,
                Err(e) => {
                    errors += 1;
                    self.spider.on_error(&req, &e.to_string()).await;
                    continue;
                }
            };

            // Check blocked
            if self.spider.is_blocked(&resp) {
                errors += 1;
                continue;
            }

            pages_crawled += 1;

            // Parse
            let (items, follow_requests) = self.spider.parse(resp).await;

            // Process items
            for item in items {
                if let Some(processed) = self.spider.on_item(item).await {
                    items_scraped += 1;
                    let _ = processed; // User handles storage in on_item
                }
            }

            // Schedule follow requests
            for follow_req in follow_requests {
                sched.push(follow_req);
            }

            // Download delay
            let delay = self.spider.download_delay();
            if delay > Duration::ZERO {
                tokio::time::sleep(delay).await;
            }
        }

        self.spider.on_close().await;

        Ok(CrawlStats {
            items_scraped,
            pages_crawled,
            errors,
            duration: start.elapsed(),
        })
    }

    async fn fetch_page(&self, client: &Client, req: &SpiderRequest) -> Result<SpiderResponse> {
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
}
