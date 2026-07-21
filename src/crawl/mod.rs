//! Spider-based crawling engine.

pub mod scheduler;
pub mod robots;
pub mod cache;
pub mod templates;
pub mod state;
pub mod items;
pub use state::CrawlState;
pub use items::{Items, JsonlWriter};

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
    /// 检测响应是否被阻塞。默认检测 BLOCKED_STATUS_CODES 中的状态码。
    /// 用户可重写以加入响应体关键词检测（如 "access denied"、"rate limit"）。
    fn is_blocked(&self, resp: &SpiderResponse) -> bool {
        BLOCKED_STATUS_CODES.contains(&resp.status)
    }
}

/// 默认阻塞状态码（参考 scrapling）：401/403/407/429/444/500/502/503/504
pub const BLOCKED_STATUS_CODES: &[u16] = &[401, 403, 407, 429, 444, 500, 502, 503, 504];

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
    /// blocked 请求总数（含重试后成功的）
    pub blocked_requests: usize,
    /// 重试次数总计
    pub retry_count: usize,
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

/// 爬取过程中的事件流
#[derive(Debug, Clone)]
pub enum CrawlEvent {
    /// 解析出一个 item
    Item(Value),
    /// 完成一页（含当前累计统计）
    PageScraped { url: String, stats: CrawlStats },
    /// 请求失败
    Error { url: String, error: String },
    /// 爬取结束（携带最终统计）
    Done(CrawlStats),
}

/// 流式爬取事件流
pub struct CrawlStream {
    inner: std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>>,
}

impl CrawlStream {
    /// 仅消费 Item 事件（最常见用法）
    pub fn items(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = Value>>> {
        use futures::StreamExt;
        Box::pin(self.inner.filter_map(|e| async move {
            match e { CrawlEvent::Item(v) => Some(v), _ => None }
        }))
    }

    /// 消费所有事件（调试/监控用）
    pub fn events(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = CrawlEvent>>> {
        self.inner
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
        self.run_with_sender(None).await
    }

    /// 流式运行：边爬边产出事件。
    ///
    /// 与 `run()` 的区别：通过 mpsc channel(128) 把 Item/PageScraped/Error/Done 事件推给消费者。
    /// run() 保持旧行为（收集 stats），stream() 额外暴露事件流。
    ///
    /// 实现说明：由于 `Engine<S>` 含 `Arc<Store>`（`Store` 内 `rusqlite::Connection`
    /// 为 `!Sync`），`Engine<S>: !Send`，故不能用 `tokio::spawn`。这里用
    /// `stream::unfold` + `tokio::select!` 在当前 task 内按需驱动 `run_with_sender`，
    /// 同时转发 channel 中的事件，实现真正的 demand-driven 流式输出。
    pub fn stream(self) -> CrawlStream {
        let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);

        let driver = async move {
            let stats = self.run_with_sender(Some(tx.clone())).await;
            match stats {
                Ok(s) => { let _ = tx.send(CrawlEvent::Done(s)).await; }
                Err(e) => {
                    let _ = tx.send(CrawlEvent::Error {
                        url: "*".into(),
                        error: e.to_string(),
                    }).await;
                    let _ = tx.send(CrawlEvent::Done(CrawlStats::default())).await;
                }
            }
        };

        let driver = Box::pin(driver);
        let rx = tokio_stream::wrappers::ReceiverStream::new(rx);

        use futures::StreamExt;
        let s = futures::stream::unfold(
            (driver, rx, false),
            |(mut driver, mut rx, driver_done)| async move {
                if driver_done {
                    return rx.next().await.map(|e| (e, (driver, rx, true)));
                }
                tokio::select! {
                    biased;
                    event = rx.next() => match event {
                        Some(e) => Some((e, (driver, rx, false))),
                        None => None,
                    },
                    _ = &mut driver => {
                        match rx.next().await {
                            Some(e) => Some((e, (driver, rx, true))),
                            None => None,
                        }
                    }
                }
            },
        );

        CrawlStream {
            inner: Box::pin(s),
        }
    }

    /// 内部：带可选事件发送器的运行逻辑。
    ///
    /// `tx=None` 时等价于原 run()（不发事件）；`tx=Some` 时在 item/页完成/错误处发事件。
    /// 此方法把 run() 的逻辑重构为可复用，run() 调用 run_with_sender(None)，stream() 调用 run_with_sender(Some(tx))。
    async fn run_with_sender(self, tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>) -> Result<CrawlStats> {
        let start = std::time::Instant::now();
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

        if let Some(ref state) = restored_state {
            for req in &state.pending_urls {
                sched.push(req.clone()).await;
            }
        } else {
            for url in start_urls {
                sched.push(SpiderRequest::get(&url)).await;
            }
        }

        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();
        let stats_items = Arc::new(AtomicUsize::new(0));
        let stats_pages = Arc::new(AtomicUsize::new(0));
        let stats_errors = Arc::new(AtomicUsize::new(0));
        let stats_blocked = Arc::new(AtomicUsize::new(0));
        let stats_retries = Arc::new(AtomicUsize::new(0));

        let domain_sems: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let follow_rx = Arc::new(Mutex::new(follow_rx));
        let client = Arc::new(client);
        let allowed = Arc::new(allowed);
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
            let stats_blocked = stats_blocked.clone();
            let stats_retries = stats_retries.clone();
            let domain_sems = domain_sems.clone();
            let robots_cache = robots_cache.clone();
            let allowed = allowed.clone();
            let in_flight = in_flight.clone();
            let tx = tx.clone();

            stream::unfold((), move |_| {
                let sched = sched.clone();
                let follow_rx = follow_rx.clone();
                let follow_tx = follow_tx.clone();
                let spider = spider.clone();
                let client = client.clone();
                let stats_pages = stats_pages.clone();
                let stats_errors = stats_errors.clone();
                let stats_items = stats_items.clone();
                let stats_blocked = stats_blocked.clone();
                let stats_retries = stats_retries.clone();
                let domain_sems = domain_sems.clone();
                let robots_cache = robots_cache.clone();
                let allowed = allowed.clone();
                let in_flight = in_flight.clone();
                let tx = tx.clone();

                async move {
                    loop {
                        let mut rx_guard = follow_rx.lock().await;
                        while let Ok(req) = rx_guard.try_recv() {
                            sched.push(req).await;
                        }
                        drop(rx_guard);

                        if stats_pages.load(Ordering::SeqCst) >= max_pages {
                            if in_flight.load(Ordering::SeqCst) == 0 {
                                return None;
                            }
                            tokio::task::yield_now().await;
                            continue;
                        }

                        let req = match sched.pop().await {
                            Some(req) => req,
                            None => {
                                if in_flight.load(Ordering::SeqCst) == 0 {
                                    return None;
                                }
                                tokio::task::yield_now().await;
                                continue;
                            }
                        };

                        in_flight.fetch_add(1, Ordering::SeqCst);
                        let spider_clone = spider.clone();
                        let stats_pages_c = stats_pages.clone();
                        let stats_errors_c = stats_errors.clone();
                        let stats_items_c = stats_items.clone();
                        let stats_blocked_c = stats_blocked.clone();
                        let stats_retries_c = stats_retries.clone();
                        let follow_tx_c = follow_tx.clone();
                        let client_c = client.clone();
                        let domain_sems_c = domain_sems.clone();
                        let robots_cache_c = robots_cache.clone();
                        let allowed_c = allowed.clone();
                        let in_flight_c = in_flight.clone();
                        let tx_c = tx.clone();

                        let fut = async move {
                            let _guard = InFlightGuard { counter: in_flight_c };

                            if !allowed_c.is_empty() {
                                if let Ok(parsed) = url::Url::parse(&req.url) {
                                    if let Some(host) = parsed.host_str() {
                                        if !allowed_c.contains(host) {
                                            return;
                                        }
                                    }
                                }
                            }

                            if obey_robots {
                                let url_clone = req.url.clone();
                                let client_r = client_c.clone();
                                let allowed_flag = {
                                    let mut rc = robots_cache_c.lock().await;
                                    rc.is_allowed(&client_r, &url_clone).await
                                };
                                if !allowed_flag {
                                    return;
                                }
                            }

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

                            let delay = spider_clone.download_delay();
                            if delay > Duration::ZERO {
                                tokio::time::sleep(delay).await;
                            }

                            let max_retries = spider_clone.max_retries();
                            let mut attempt: u32 = 0;
                            let mut last_error: Option<String> = None;
                            let mut final_resp: Option<SpiderResponse> = None;

                            loop {
                                attempt += 1;
                                match fetch_page(&client_c, &req).await {
                                    Ok(resp) => {
                                        if spider_clone.is_blocked(&resp) {
                                            stats_blocked_c.fetch_add(1, Ordering::SeqCst);
                                            if attempt <= max_retries {
                                                stats_retries_c.fetch_add(1, Ordering::SeqCst);
                                                let delay = spider_clone.download_delay();
                                                if delay > Duration::ZERO {
                                                    tokio::time::sleep(delay).await;
                                                }
                                                tracing::warn!(
                                                    "blocked (status={}, attempt={}/{}), retrying: {}",
                                                    resp.status, attempt, max_retries, req.url
                                                );
                                                continue;
                                            }
                                            // 重试次数耗尽
                                            stats_errors_c.fetch_add(1, Ordering::SeqCst);
                                            last_error = Some(format!(
                                                "blocked after {} retries (status={})",
                                                max_retries, resp.status
                                            ));
                                            break;
                                        }
                                        // 成功
                                        final_resp = Some(resp);
                                        break;
                                    }
                                    Err(e) => {
                                        if attempt <= max_retries {
                                            stats_retries_c.fetch_add(1, Ordering::SeqCst);
                                            let delay = spider_clone.download_delay();
                                            if delay > Duration::ZERO {
                                                tokio::time::sleep(delay).await;
                                            }
                                            tracing::warn!(
                                                "fetch error (attempt={}/{}): {} - {}",
                                                attempt, max_retries, e, req.url
                                            );
                                            continue;
                                        }
                                        stats_errors_c.fetch_add(1, Ordering::SeqCst);
                                        spider_clone.on_error(&req, &e.to_string()).await;
                                        last_error = Some(e.to_string());
                                        break;
                                    }
                                }
                            }

                            // 处理最终结果
                            if let Some(resp) = final_resp {
                                stats_pages_c.fetch_add(1, Ordering::SeqCst);
                                let page_url = resp.url.clone();
                                let (items, follows) = spider_clone.parse(resp).await;
                                for item in items {
                                    if let Some(processed) = spider_clone.on_item(item).await {
                                        stats_items_c.fetch_add(1, Ordering::SeqCst);
                                        if let Some(ref tx) = tx_c {
                                            let _ = tx.send(CrawlEvent::Item(processed)).await;
                                        }
                                    }
                                }
                                for f in follows {
                                    let _ = follow_tx_c.send(f);
                                }
                                if let Some(ref tx) = tx_c {
                                    let _ = tx.send(CrawlEvent::PageScraped {
                                        url: page_url,
                                        stats: CrawlStats {
                                            items_scraped: stats_items_c.load(Ordering::SeqCst),
                                            pages_crawled: stats_pages_c.load(Ordering::SeqCst),
                                            errors: stats_errors_c.load(Ordering::SeqCst),
                                            duration: start.elapsed(),
                                            blocked_requests: stats_blocked_c.load(Ordering::SeqCst),
                                            retry_count: stats_retries_c.load(Ordering::SeqCst),
                                            ..Default::default()
                                        },
                                    }).await;
                                }
                            } else if let Some(err) = last_error {
                                if let Some(ref tx) = tx_c {
                                    let _ = tx.send(CrawlEvent::Error {
                                        url: req.url.clone(),
                                        error: err,
                                    }).await;
                                }
                            }
                        };

                        return Some((fut, ()));
                    }
                }
            })
            .buffer_unordered(max_concurrent)
        };

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
            blocked_requests: stats_blocked.load(Ordering::SeqCst),
            retry_count: stats_retries.load(Ordering::SeqCst),
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
    fn test_blocked_status_codes_contains_common_codes() {
        assert!(BLOCKED_STATUS_CODES.contains(&401));
        assert!(BLOCKED_STATUS_CODES.contains(&403));
        assert!(BLOCKED_STATUS_CODES.contains(&407));
        assert!(BLOCKED_STATUS_CODES.contains(&429));
        assert!(BLOCKED_STATUS_CODES.contains(&444));
        assert!(BLOCKED_STATUS_CODES.contains(&500));
        assert!(BLOCKED_STATUS_CODES.contains(&502));
        assert!(BLOCKED_STATUS_CODES.contains(&503));
        assert!(BLOCKED_STATUS_CODES.contains(&504));
        assert!(!BLOCKED_STATUS_CODES.contains(&200));
        assert!(!BLOCKED_STATUS_CODES.contains(&301));
        assert!(!BLOCKED_STATUS_CODES.contains(&404));
    }

    #[test]
    fn test_spider_default_is_blocked_detects_status_codes() {
        struct DummySpider;
        #[async_trait]
        impl Spider for DummySpider {
            fn name(&self) -> &str { "dummy" }
            fn start_urls(&self) -> Vec<String> { vec![] }
            async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) { (vec![], vec![]) }
        }
        let spider = DummySpider;
        let blocked_resp = SpiderResponse {
            url: "http://example.com".into(),
            status: 403,
            headers: HashMap::new(),
            body: vec![],
            request: SpiderRequest::get("http://example.com"),
        };
        assert!(spider.is_blocked(&blocked_resp));
        let ok_resp = SpiderResponse { status: 200, ..blocked_resp };
        assert!(!spider.is_blocked(&ok_resp));
    }

    /// 启动一个本地 HTTP 测试服务器，对任意请求返回 `html`，返回其 base URL。
    ///
    /// 计划原用 `data:text/html,...` URL，但 wreq HTTP fetcher 不支持 data URI，
    /// 故改用本地 TCP 服务器提供相同 HTML 内容。
    async fn spawn_html_server(html: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let html = html;
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(), html
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://{}", addr)
    }

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
            ..Default::default()
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

    #[tokio::test]
    async fn test_stream_emits_item_and_done() {
        use async_trait::async_trait;
        use std::collections::HashSet;

        let base = spawn_html_server("<p>1</p>").await;

        struct CountSpider { start_url: String }
        #[async_trait]
        impl Spider for CountSpider {
            fn name(&self) -> &str { "count" }
            fn start_urls(&self) -> Vec<String> { vec![self.start_url.clone()] }
            async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
                let node = resp.parse().unwrap();
                let text = node.select("p").text().join("");
                (vec![serde_json::json!({"text": text})], vec![])
            }
            fn obey_robots(&self) -> bool { false }
        }

        let engine = Engine::new(CountSpider { start_url: base }).max_pages(1);
        let mut stream = engine.stream().events();
        use futures::StreamExt;
        let mut items = 0;
        let mut done = false;
        while let Some(event) = stream.next().await {
            match event {
                CrawlEvent::Item(_) => items += 1,
                CrawlEvent::Done(stats) => {
                    assert!(stats.pages_crawled >= 1);
                    done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(done, "应收到 Done 事件");
        assert!(items >= 1, "应至少收到 1 个 Item 事件, 实际 {}", items);
    }

    #[tokio::test]
    async fn test_stream_items_helper() {
        use async_trait::async_trait;

        let base = spawn_html_server("<p>hello</p>").await;

        struct OneSpider { start_url: String }
        #[async_trait]
        impl Spider for OneSpider {
            fn name(&self) -> &str { "one" }
            fn start_urls(&self) -> Vec<String> { vec![self.start_url.clone()] }
            async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
                (vec![serde_json::json!({"v": 1})], vec![])
            }
            fn obey_robots(&self) -> bool { false }
        }

        let engine = Engine::new(OneSpider { start_url: base }).max_pages(1);
        let mut items_stream = engine.stream().items();
        use futures::StreamExt;
        let mut count = 0;
        while items_stream.next().await.is_some() {
            count += 1;
        }
        assert!(count >= 1, "items() 应产出至少 1 个 item");
    }
}
