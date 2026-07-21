//! Spider-based crawling engine.

pub mod scheduler;
pub mod robots;
pub mod cache;
pub mod templates;
pub mod state;
pub use state::CrawlState;

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;

use crate::error::{WispError, Result};
use crate::fetch::{self, Client};
use crate::parser::Node;

/// HTTP method for spider requests.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Method { Get, Post, Put, Delete }

/// A request to be processed by the spider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiderRequest {
    pub url: String,
    pub method: Method,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    // `serde_json::Value` 的 Deserialize 依赖 `deserialize_any`，bincode 不支持。
    // checkpoint 场景下 `meta` 当前不被读取，跳过它以让 bincode round-trip 可行。
    // 注意：`#[serde(skip)]` 对所有 Serializer 生效（含 serde_json），未来若用
    // serde_json 序列化 SpiderRequest 需重新评估（改用 `#[serde(with = "...")]`）。
    #[serde(skip)]
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
#[derive(Debug, Clone, Default)]
pub struct CrawlStats {
    pub items_scraped: usize,
    pub pages_crawled: usize,
    pub errors: usize,
    pub duration: Duration,
    /// 总下载字节数（响应体累加）
    pub bytes_downloaded: u64,
    /// 平均响应时间
    pub avg_response_time: Duration,
    /// 每域名页数
    pub domain_counts: HashMap<String, usize>,
}

impl CrawlStats {
    /// 打印人类可读的统计摘要
    pub fn summary(&self) -> String {
        format!(
            "爬取完成: {} 页 / {} items / {} 错误 / 耗时 {:?} / {:.1} KB / 平均响应 {:?}",
            self.pages_crawled,
            self.items_scraped,
            self.errors,
            self.duration,
            self.bytes_downloaded as f64 / 1024.0,
            self.avg_response_time
        )
    }
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
    checkpoint_store: Option<Arc<crate::storage::Store>>,
    checkpoint_interval: usize,
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
            checkpoint_store: None,
            checkpoint_interval: 100,
        }
    }

    pub fn max_pages(mut self, n: usize) -> Self { self.config.max_pages = n; self }
    pub fn max_concurrent(mut self, n: usize) -> Self { self.config.max_concurrent = n; self }

    pub fn with_checkpoint(mut self, store: Arc<crate::storage::Store>) -> Self {
        self.checkpoint_store = Some(store);
        self
    }

    pub fn checkpoint_interval(mut self, n: usize) -> Self {
        self.checkpoint_interval = n;
        self
    }

    pub async fn run(self) -> Result<CrawlStats> {
        let start = std::time::Instant::now();
        // 提前提取所有需要的信息（避免 self 部分移动问题）
        let max_pages = self.config.max_pages;
        let max_concurrent = self.config.max_concurrent;
        let obey_robots = self.spider.obey_robots();
        let allowed = self.spider.allowed_domains();
        let start_urls = self.spider.start_urls();
        let fetcher_config = self.spider.fetcher_config();
        let checkpoint_store = self.checkpoint_store.clone();
        let checkpoint_interval = self.checkpoint_interval;
        let spider_name = self.spider.name().to_string();

        let client = Client::builder()
            .timeout(fetcher_config.timeout)
            .build()?;

        // === checkpoint 恢复 ===
        let mut restored_state: Option<CrawlState> = None;
        if let Some(ref store) = checkpoint_store {
            if let Some(blob) = store.load_checkpoint(&spider_name)? {
                match bincode::deserialize::<CrawlState>(&blob) {
                    Ok(state) => {
                        tracing::info!(
                            "恢复 checkpoint: {} 个待爬 URL, {} 个已访问",
                            state.pending_urls.len(), state.seen_urls.len()
                        );
                        restored_state = Some(state);
                    }
                    Err(e) => {
                        tracing::warn!("checkpoint 反序列化失败，将重新开始: {}", e);
                    }
                }
            }
        }

        self.spider.on_start().await;

        let spider = Arc::new(self.spider);
        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(Mutex::new(robots::RobotsCache::new()));

        // Seed start URLs (or restore from checkpoint)
        if let Some(ref state) = restored_state {
            for req in &state.pending_urls {
                sched.push(req.clone()).await;
            }
            // stage 1: seen_urls 不单独恢复（Scheduler 的 seen_urls 是 placeholder）
        } else {
            for url in start_urls {
                sched.push(SpiderRequest::get(&url)).await;
            }
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
        // C1 fix: track in-flight futures so unfold does not terminate while
        // running futures may still emit follow requests into the channel.
        let in_flight = Arc::new(AtomicUsize::new(0));

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
            let in_flight = in_flight.clone();

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
                let in_flight = in_flight.clone();

                async move {
                    loop {
                        // 1. Drain follow channel into scheduler
                        let mut rx_guard = follow_rx.lock().await;
                        while let Ok(req) = rx_guard.try_recv() {
                            sched.push(req).await;
                        }
                        drop(rx_guard);

                        // 2. Check page budget
                        if stats_pages.load(Ordering::SeqCst) >= max_pages {
                            // Budget reached: don't start new fetches. Wait for all
                            // in-flight futures to finish so their follow requests can
                            // still be drained and counted (C1 fix).
                            if in_flight.load(Ordering::SeqCst) == 0 {
                                return None;
                            }
                            tokio::task::yield_now().await;
                            continue;
                        }

                        // 3. Pop next request
                        let req = match sched.pop().await {
                            Some(req) => req,
                            None => {
                                // Scheduler empty: if no in-flight futures, truly done;
                                // otherwise wait for them to emit follow requests (C1 fix).
                                if in_flight.load(Ordering::SeqCst) == 0 {
                                    return None;
                                }
                                tokio::task::yield_now().await;
                                continue;
                            }
                        };

                        // 4-7. All logic in a single async block (unified future type)
                        in_flight.fetch_add(1, Ordering::SeqCst);
                        let spider_clone = spider.clone();
                        let stats_pages_c = stats_pages.clone();
                        let stats_errors_c = stats_errors.clone();
                        let stats_items_c = stats_items.clone();
                        let follow_tx_c = follow_tx.clone();
                        let client_c = client.clone();
                        let domain_sems_c = domain_sems.clone();
                        let robots_cache_c = robots_cache.clone();
                        let allowed_c = allowed.clone();
                        let in_flight_c = in_flight.clone();

                        let fut = async move {
                            // RAII guard: ensures in_flight is decremented on every exit
                            // path (early return, error, normal completion). Holds an Arc
                            // clone to avoid self-referential async state machine.
                            let _guard = InFlightGuard { counter: in_flight_c };

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
                            // NOTE (I2): robots_cache 的全局 Mutex 在 is_allowed 的网络拉取
                            // 期间被持有，序列化所有域的 robots 检查。阶段 1 接受此性能限制，
                            // 后续可改为 per-domain 锁或在 RobotsCache 内部双检。
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
                                sems.entry(domain)
                                    .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
                                    .clone()
                            };
                            let _permit = sem.acquire_owned().await.unwrap();

                            // I1 fix: download_delay - per-domain 信号量 acquire 后、fetch 前
                            let delay = spider_clone.download_delay();
                            if delay > Duration::ZERO {
                                tokio::time::sleep(delay).await;
                            }

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
                        return Some((fut, ()));
                    }
                }
            })
            .buffer_unordered(max_concurrent)
        };

        // Drive the stream to completion
        tokio::pin!(stream);
        let mut pages_since_checkpoint = 0usize;
        while stream.next().await.is_some() {
            pages_since_checkpoint += 1;
            if pages_since_checkpoint >= checkpoint_interval {
                if let Some(ref store) = checkpoint_store {
                    let pending = sched.pending_urls().await;
                    let snapshot_stats = CrawlStats {
                        items_scraped: stats_items.load(Ordering::SeqCst),
                        pages_crawled: stats_pages.load(Ordering::SeqCst),
                        errors: stats_errors.load(Ordering::SeqCst),
                        duration: start.elapsed(),
                        ..Default::default()
                    };
                    let state = CrawlState::from_stats(
                        spider_name.clone(),
                        &snapshot_stats,
                        pending,
                    );
                    match bincode::serialize(&state) {
                        Ok(blob) => {
                            if let Err(e) = store.save_checkpoint(
                                &spider_name,
                                &blob,
                                state.saved_at.timestamp(),
                            ) {
                                tracing::warn!("checkpoint 保存失败: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("checkpoint 序列化失败: {}", e);
                        }
                    }
                }
                pages_since_checkpoint = 0;
            }
        }

        spider.on_close().await;

        // === checkpoint 清理：爬取正常完成，删除 checkpoint ===
        if let Some(ref store) = checkpoint_store {
            if let Err(e) = store.delete_checkpoint(&spider_name) {
                tracing::warn!("删除 checkpoint 失败: {}", e);
            }
        }

        Ok(CrawlStats {
            items_scraped: stats_items.load(Ordering::SeqCst),
            pages_crawled: stats_pages.load(Ordering::SeqCst),
            errors: stats_errors.load(Ordering::SeqCst),
            duration: start.elapsed(),
            ..Default::default()
        })
    }
}

/// RAII guard for in-flight future counting (C1 fix).
///
/// Constructed after `in_flight.fetch_add(1, ...)`; `Drop` runs `fetch_sub(1, ...)`
/// so the counter is always decremented regardless of how the future exits
/// (early return, normal completion, or error). Holds an `Arc` clone rather than
/// a `&AtomicUsize` reference to avoid a self-referential async state machine.
struct InFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_crawl_stats_summary() {
        let stats = CrawlStats {
            items_scraped: 10,
            pages_crawled: 5,
            errors: 1,
            duration: Duration::from_secs(30),
            bytes_downloaded: 2048,
            avg_response_time: Duration::from_millis(500),
            domain_counts: {
                let mut m = HashMap::new();
                m.insert("example.com".to_string(), 5);
                m
            },
        };
        let s = stats.summary();
        assert!(s.contains("5 页"), "summary 应含页数: {}", s);
        assert!(s.contains("10 items"), "summary 应含 items: {}", s);
        assert!(s.contains("1 错误"), "summary 应含错误数: {}", s);
        assert!(s.contains("2.0 KB"), "summary 应含字节数: {}", s);
    }

    #[test]
    fn test_crawl_stats_default() {
        let stats = CrawlStats::default();
        assert_eq!(stats.items_scraped, 0);
        assert_eq!(stats.bytes_downloaded, 0);
        assert!(stats.domain_counts.is_empty());
        assert_eq!(stats.avg_response_time, Duration::ZERO);
    }
}
