//! Engine 实现 - 从 mod.rs 拆分，降低圈复杂度。
//!
//! 核心拆解：
//! - `EngineContext` 打包所有共享状态（替代 20+ 个 Arc 变量传递）
//! - `process_request()` 处理单个请求（替代 200 行嵌套闭包）
//! - `fetch_with_retry()` 重试循环
//! - `auto_upgrade_check()` Auto 模式升级检查

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::error::Result;
use crate::http::{self, Client};
use crate::fetcher::FetchMode;
use super::{
    Spider, SpiderRequest, SpiderResponse, Method,
    CrawlStats, CrawlEvent, CrawlState,
    auto, scheduler, robots,
};
use super::stats::SpiderStats;

// === EngineContext: 打包所有共享状态 ===

/// Engine 运行时共享上下文（所有 Spider 共用）。
///
/// 共享字段：client / sched / robots_cache / follow_tx / follow_rx / domain_sems /
/// proxy_pool / cache_store / request_cache / abort_flag / start / tx / dev_mode。
/// per-spider 字段以 Vec 数组持有，路由时按下标 `idx` 取值。
pub(crate) struct EngineContext {
    pub client: Arc<Client>,
    pub sched: Arc<scheduler::Scheduler>,
    pub robots_cache: Arc<Mutex<robots::RobotsCache>>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<SpiderRequest>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<SpiderRequest>>>,
    pub domain_sems: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>,
    pub proxy_pool: Option<Arc<crate::proxy::ProxyPool>>,
    pub cache_store: Option<Arc<crate::storage::Store>>,
    pub request_cache: Option<super::request_cache::RequestCache>,
    pub abort_flag: Arc<AtomicBool>,
    pub start: std::time::Instant,
    pub tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    pub dev_mode: bool,
    // === per-spider 配置与统计（引擎持有多个，路由时选一个传入）===
    pub spiders: Vec<Arc<dyn Spider>>,
    pub stats: Vec<Arc<SpiderStats>>,
    pub rule_engines: Vec<Arc<Mutex<auto::ModeRuleEngine>>>,
    pub auto_excludes: Vec<HashSet<String>>,
    pub allowed_list: Vec<Arc<HashSet<String>>>,
    pub fetcher_configs: Vec<http::Config>,
    pub fetch_modes: Vec<FetchMode>,
    pub max_concurrents: Vec<usize>,
    pub max_depths: Vec<u32>,
    pub obey_robots_flags: Vec<bool>,
    pub global_in_flight: Arc<AtomicUsize>,
    pub engine_max_pages: usize,
    /// 预编译的 per-spider 正则模式（路由用，避免每次 matches 重新编译）。
    pub compiled_patterns: Vec<Vec<regex::Regex>>,
}

// === 核心函数：处理单个请求 ===

/// 处理单个请求的完整流程：域名过滤 → 深度检查 → 缓存 → robots → 信号量 → 延迟 → 重试 → Auto 检查 → parse。
/// idx 为命中的 Spider 在 spiders/stats 数组中的下标。
pub(crate) async fn process_request(ctx: &EngineContext, req: SpiderRequest, idx: usize) {
    let spider = &ctx.spiders[idx];
    let stats = &ctx.stats[idx];
    let allowed = &ctx.allowed_list[idx];
    let max_depth = ctx.max_depths[idx];
    let max_concurrent = ctx.max_concurrents[idx];
    let obey_robots = ctx.obey_robots_flags[idx];

    // 1. 域名过滤
    if !allowed.is_empty() {
        if let Ok(parsed) = url::Url::parse(&req.url) {
            if let Some(host) = parsed.host_str() {
                if !allowed.contains(host) {
                    stats.offsite.fetch_add(1, Ordering::SeqCst);
                    return;
                }
            }
        }
    }

    // 1.5. 深度检查
    if req.depth > max_depth {
        return;
    }

    // 1.6. 全局控制函数检查
    if super::control::is_cancelled(&req.url).await { return; }
    if !super::control::wait_if_paused(&req.url).await { return; }
    if super::control::is_shutdown() { return; }

    // 1.7. 异步钩子检查
    match spider.on_before_request(&req).await {
        super::RequestAction::Proceed => {},
        super::RequestAction::Skip => { return; },
        super::RequestAction::Delay(d) => { tokio::time::sleep(d).await; },
        super::RequestAction::Abort => {
            ctx.abort_flag.store(true, Ordering::SeqCst);
            return;
        }
    }

    // 2. 内存缓存检查 (RequestCache)
    if let Some(ref rc) = ctx.request_cache {
        if let Some(entry) = rc.get(&req.url).await {
            let resp = SpiderResponse {
                url: req.url.clone(),
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
                request: req.clone(),
                tracker: None,
                from_cache: true,
            };
            stats.cache_hits.fetch_add(1, Ordering::SeqCst);
            record_status(stats, resp.status).await;
            // 直接跳到处理结果阶段
            return process_response(ctx, resp, &req, idx).await;
        }
    }

    // 3. 开发模式 SQLite 缓存检查
    let method_str = match req.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    };
    let cached_resp: Option<crate::storage::CachedResponse> = if ctx.dev_mode {
        ctx.cache_store.as_ref().and_then(|s| {
            s.load_cached_response(&req.url, method_str).ok().flatten()
        })
    } else {
        None
    };

    let mut final_resp: Option<SpiderResponse> = None;
    let mut last_error: Option<String> = None;

    if let Some(cached) = cached_resp {
        // 命中缓存
        let resp = SpiderResponse {
            url: req.url.clone(),
            status: cached.status,
            headers: cached.headers,
            body: cached.body,
            request: req.clone(),
            tracker: None,
            from_cache: true,
        };
        stats.cache_hits.fetch_add(1, Ordering::SeqCst);
        record_status(stats, resp.status).await;
        final_resp = Some(resp);
    } else {
        // 3. Robots 检查
        if obey_robots {
            let allowed_flag = {
                let mut rc = ctx.robots_cache.lock().await;
                rc.is_allowed(&ctx.client, &req.url).await
            };
            if !allowed_flag { return; }
        }

        // 4. 域名信号量
        let domain = url::Url::parse(&req.url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let sem = {
            let mut sems = ctx.domain_sems.lock().await;
            sems.entry(domain)
                .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
                .clone()
        };
        let _permit = sem.acquire_owned().await.unwrap();

        // 5. 延迟
        apply_delay(ctx, &req.url, spider, obey_robots).await;

        // 6. 带重试的抓取
        let (resp, err) = fetch_with_retry(ctx, &req, idx).await;
        final_resp = resp;
        last_error = err;

        // 7. 开发模式缓存保存
        if ctx.dev_mode {
            if let Some(ref store) = ctx.cache_store {
                if let Some(ref resp) = final_resp {
                    let cached = crate::storage::CachedResponse {
                        status: resp.status,
                        headers: resp.headers.clone(),
                        body: resp.body.clone(),
                        cached_at: chrono::Utc::now().timestamp(),
                    };
                    let _ = store.save_cached_response(&req.url, method_str, &cached);
                }
            }
        }

        // 7.5. 写入 RequestCache
        if let Some(ref rc) = ctx.request_cache {
            if let Some(ref resp) = final_resp {
                rc.put(&req.url, super::request_cache::CachedEntry {
                    status: resp.status,
                    headers: resp.headers.clone(),
                    body: resp.body.clone(),
                }).await;
            }
        }
    }

    // 8. 处理结果
    if let Some(resp) = final_resp {
        process_response(ctx, resp, &req, idx).await;
    } else if let Some(err) = last_error {
        if let Some(ref tx) = ctx.tx {
            let _ = tx.send(CrawlEvent::Error { url: req.url.clone(), error: err }).await;
        }
    }
}

/// 处理已获取的响应：parse → Auto 升级 → items → events。
pub(crate) async fn process_response(ctx: &EngineContext, resp: SpiderResponse, req: &SpiderRequest, idx: usize) {
    let spider = &ctx.spiders[idx];
    let stats = &ctx.stats[idx];

    if !resp.from_cache {
        stats.pages.fetch_add(1, Ordering::SeqCst);
    }
    let page_url = resp.url.clone();

    let tracker_ref = resp.tracker.clone();
    let (mut items, mut follows) = spider.parse(resp).await;

    // Auto 升级检查
    if ctx.fetch_modes[idx] == FetchMode::Auto {
        if let Some(result) = auto_upgrade_check(ctx, &tracker_ref, &page_url, req, idx).await {
            items = result.0;
            follows = result.1;
        }
    }

    // 发送 items
    for item in items {
        if let Some(processed) = spider.on_item(item).await {
            stats.items.fetch_add(1, Ordering::SeqCst);
            if let Some(ref tx) = ctx.tx {
                let _ = tx.send(CrawlEvent::Item(processed)).await;
            }
        }
    }
    for f in follows {
        let _ = ctx.follow_tx.send(f);
    }

    // PageScraped 事件
    if let Some(ref tx) = ctx.tx {
        let status_codes_snapshot = stats.status_codes.lock().await.clone();
        let _ = tx.send(CrawlEvent::PageScraped {
            url: page_url,
            stats: snapshot_stats_for(stats, status_codes_snapshot, ctx.start),
        }).await;
    }
}

// === 带重试的抓取 ===

/// 重试循环：fetch → blocked 检测 → 重试/成功/失败。
async fn fetch_with_retry(ctx: &EngineContext, req: &SpiderRequest, idx: usize) -> (Option<SpiderResponse>, Option<String>) {
    let spider = &ctx.spiders[idx];
    let stats = &ctx.stats[idx];
    let fetch_mode = ctx.fetch_modes[idx];
    let fetcher_config = &ctx.fetcher_configs[idx];
    let rule_engine = &ctx.rule_engines[idx];
    let max_retries = spider.max_retries();
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        let proxy = ctx.proxy_pool.as_ref().and_then(|p| p.next());
        match fetch_page(&ctx.client, req, proxy.as_deref(), fetch_mode, fetcher_config, rule_engine).await {
            Ok(resp) => {
                record_status(stats, resp.status).await;
                if spider.is_blocked(&resp) {
                    stats.blocked.fetch_add(1, Ordering::SeqCst);
                    // attempt 从 1 起；attempt <= max_retries 表示还能重试，
                    // 故 max_retries=3 时实际尝试 4 次（attempt 1..=4）。
                    if attempt <= max_retries {
                        stats.retries.fetch_add(1, Ordering::SeqCst);
                        let delay = spider.download_delay();
                        if delay > Duration::ZERO { tokio::time::sleep(delay).await; }
                        tracing::warn!("blocked (status={}, attempt={}/{}), retrying: {}", resp.status, attempt, max_retries, req.url);
                        continue;
                    }
                    stats.errors.fetch_add(1, Ordering::SeqCst);
                    return (None, Some(format!(
                        "blocked after {} retries (status={}, total attempts={})",
                        max_retries, resp.status, attempt
                    )));
                }
                return (Some(resp), None);
            }
            Err(e) => {
                if attempt <= max_retries {
                    stats.retries.fetch_add(1, Ordering::SeqCst);
                    let delay = spider.download_delay();
                    if delay > Duration::ZERO { tokio::time::sleep(delay).await; }
                    tracing::warn!("fetch error (attempt={}/{}): {} - {}", attempt, max_retries, e, req.url);
                    continue;
                }
                stats.errors.fetch_add(1, Ordering::SeqCst);
                spider.on_error(req, &e.to_string()).await;
                return (None, Some(format!(
                    "fetch failed after {} retries (total attempts={}): {}",
                    max_retries, attempt, e
                )));
            }
        }
    }
}

// === Auto 升级检查 ===

/// Auto 模式：检查 tracker 中选择器是否有 0 匹配，若有则升级 Dynamic 重取。
/// 返回 Some((items, follows)) 表示已升级并重新 parse；None 表示无需升级。
async fn auto_upgrade_check(
    ctx: &EngineContext,
    tracker: &Option<Arc<std::sync::Mutex<auto::SelectorTracker>>>,
    page_url: &str,
    req: &SpiderRequest,
    idx: usize,
) -> Option<(Vec<Value>, Vec<SpiderRequest>)> {
    let spider = &ctx.spiders[idx];
    let fetcher_config = &ctx.fetcher_configs[idx];
    let rule_engine = &ctx.rule_engines[idx];
    let auto_exclude = &ctx.auto_excludes[idx];
    let tracker = tracker.as_ref()?;
    let needs = tracker.lock().unwrap().needs_upgrade(auto_exclude);

    if needs {
        {
            let mut engine = rule_engine.lock().await;
            engine.learn(page_url, FetchMode::Dynamic);
        }
        tracing::info!("Auto: '{}' 选择器无内容，升级 Dynamic", page_url);
        let proxy = ctx.proxy_pool.as_ref().and_then(|p| p.next());
        let dynamic_resp = fetch_page_inner(
            &ctx.client, req, proxy.as_deref(), FetchMode::Dynamic, fetcher_config,
        ).await;
        if let Ok(new_resp) = dynamic_resp {
            let (items, follows) = spider.parse(new_resp).await;
            return Some((items, follows));
        }
        None
    } else {
        let mut engine = rule_engine.lock().await;
        engine.learn(page_url, FetchMode::Http);
        None
    }
}

// === 辅助函数 ===

async fn record_status(stats: &Arc<SpiderStats>, status: u16) {
    let mut m = stats.status_codes.lock().await;
    *m.entry(status).or_insert(0) += 1;
}

async fn apply_delay(ctx: &EngineContext, url: &str, spider: &Arc<dyn Spider>, obey_robots: bool) {
    let mut delay = spider.download_delay();
    if obey_robots {
        let robots_delay = {
            let mut rc = ctx.robots_cache.lock().await;
            rc.crawl_delay(&ctx.client, url).await
        };
        if let Some(secs) = robots_delay {
            let robots_dur = Duration::from_secs_f64(secs);
            if robots_dur > delay { delay = robots_dur; }
        }
    }
    if delay > Duration::ZERO {
        tokio::time::sleep(delay).await;
    }
}

/// 从单个 SpiderStats 构造 CrawlStats 快照。
pub(crate) fn snapshot_stats_for(
    stats: &Arc<SpiderStats>,
    status_codes: HashMap<u16, usize>,
    start: std::time::Instant,
) -> CrawlStats {
    CrawlStats {
        items_scraped: stats.items.load(Ordering::SeqCst),
        pages_crawled: stats.pages.load(Ordering::SeqCst),
        errors: stats.errors.load(Ordering::SeqCst),
        duration: start.elapsed(),
        blocked_requests: stats.blocked.load(Ordering::SeqCst),
        retry_count: stats.retries.load(Ordering::SeqCst),
        status_code_counts: status_codes,
        offsite_requests_count: stats.offsite.load(Ordering::SeqCst),
        cache_hits: stats.cache_hits.load(Ordering::SeqCst),
        ..Default::default()
    }
}

/// Checkpoint 保存（仅单 Spider 时调用）。
pub(crate) async fn save_checkpoint(
    store: &crate::storage::Store,
    spider_name: &str,
    sched: &scheduler::Scheduler,
    stats: &Arc<SpiderStats>,
) {
    let pending = sched.pending_urls().await;
    let snapshot = snapshot_stats_for(stats, HashMap::new(), stats.start);
    let state = CrawlState::from_stats(
        spider_name.to_string(),
        &snapshot,
        pending,
    );
    match bincode::serialize(&state) {
        Ok(blob) => {
            if let Err(e) = store.save_checkpoint(spider_name, &blob, state.saved_at.timestamp()) {
                tracing::warn!("checkpoint 保存失败: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("checkpoint 序列化失败: {}", e);
        }
    }
}

// === fetch_page（模式分发）===

pub(crate) async fn fetch_page(
    client: &Client,
    req: &SpiderRequest,
    proxy_url: Option<&str>,
    mode: FetchMode,
    config: &http::Config,
    rule_engine: &Mutex<auto::ModeRuleEngine>,
) -> Result<SpiderResponse> {
    if mode == FetchMode::Auto {
        let resolved = { rule_engine.lock().await.resolve(&req.url) };
        if let Some(cached_mode) = resolved {
            return fetch_page_inner(client, req, proxy_url, cached_mode, config).await;
        }
        let resp = fetch_page_inner(client, req, proxy_url, FetchMode::Http, config).await?;
        if auto::is_blocked_response(resp.status, &resp.body, &resp.headers) {
            rule_engine.lock().await.learn(&req.url, FetchMode::Stealth);
            return fetch_page_inner(client, req, proxy_url, FetchMode::Stealth, config).await;
        }
        let tracker = Arc::new(std::sync::Mutex::new(auto::SelectorTracker::new()));
        return Ok(SpiderResponse { tracker: Some(tracker), ..resp });
    }
    fetch_page_inner(client, req, proxy_url, mode, config).await
}

/// 内部实际抓取（根据模式分发）。
pub(crate) async fn fetch_page_inner(
    client: &Client,
    req: &SpiderRequest,
    proxy_url: Option<&str>,
    mode: FetchMode,
    config: &http::Config,
) -> Result<SpiderResponse> {
    if mode == FetchMode::Dynamic || mode == FetchMode::Stealth {
        let mut builder = match mode {
            FetchMode::Dynamic => crate::fetcher::Fetcher::dynamic(),
            FetchMode::Stealth => crate::fetcher::Fetcher::stealth(),
            _ => unreachable!(),
        };
        builder = builder.timeout(config.timeout);
        if let Some(proxy) = proxy_url {
            builder = builder.proxy(proxy);
        }
        let resp = builder.get(&req.url).await?;
        return Ok(SpiderResponse {
            url: resp.url.clone(),
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body.clone(),
            request: req.clone(),
            tracker: None,
            from_cache: false,
        });
    }

    // Http 模式
    let effective_client: Client;
    let need_custom_client = proxy_url.is_some() || config.rotate_ua;
    let use_client = if need_custom_client {
        let mut builder = Client::builder()
            .timeout(client.config_ref().timeout);
        if let Some(proxy) = proxy_url {
            builder = builder.proxy(proxy);
        }
        if config.rotate_ua {
            let rotator = crate::http::UaRotator::desktop();
            builder = builder.user_agent(rotator.next());
        }
        effective_client = builder.build()?;
        &effective_client
    } else {
        client
    };

    let resp = match req.method {
        Method::Get => use_client.get(&req.url).await?,
        Method::Post => use_client.post(&req.url, req.body.as_deref(), None).await?,
        Method::Put => use_client.put(&req.url, req.body.as_deref(), None).await?,
        Method::Delete => use_client.delete(&req.url).await?,
    };

    Ok(SpiderResponse {
        url: resp.url.clone(),
        status: resp.status,
        headers: resp.headers.clone(),
        body: resp.body.clone(),
        request: req.clone(),
        tracker: None,
        from_cache: false,
    })
}

// === InFlightGuard ===

pub(crate) struct InFlightGuard {
    pub counter: Arc<AtomicUsize>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::time::Instant;

    /// 最小 Spider：parse 返回空，不产出 items/follows，避免触碰事件通道。
    struct DummySpider;

    #[async_trait]
    impl Spider for DummySpider {
        fn name(&self) -> &str { "dummy" }
        fn start_urls(&self) -> Vec<String> { vec![] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            (vec![], vec![])
        }
    }

    /// 构造最小 EngineContext（单 Spider，Http 模式，无事件通道）。
    /// 返回上下文与对应 stats 的 Arc 克隆，便于测试断言计数器。
    fn make_ctx() -> (EngineContext, Arc<SpiderStats>) {
        let stats = Arc::new(SpiderStats::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();
        let ctx = EngineContext {
            client: Arc::new(Client::new().expect("build http client")),
            sched: Arc::new(scheduler::Scheduler::new()),
            robots_cache: Arc::new(Mutex::new(robots::RobotsCache::new())),
            follow_tx,
            follow_rx: Arc::new(Mutex::new(follow_rx)),
            domain_sems: Arc::new(Mutex::new(HashMap::new())),
            proxy_pool: None,
            cache_store: None,
            request_cache: None,
            abort_flag: Arc::new(AtomicBool::new(false)),
            start: Instant::now(),
            tx: None,
            dev_mode: false,
            spiders: vec![Arc::new(DummySpider) as Arc<dyn Spider>],
            stats: vec![stats.clone()],
            rule_engines: vec![Arc::new(Mutex::new(auto::ModeRuleEngine::new()))],
            auto_excludes: vec![HashSet::new()],
            allowed_list: vec![Arc::new(HashSet::new())],
            fetcher_configs: vec![http::Config::default()],
            fetch_modes: vec![FetchMode::Http],
            max_concurrents: vec![8],
            max_depths: vec![u32::MAX],
            obey_robots_flags: vec![false],
            global_in_flight: Arc::new(AtomicUsize::new(0)),
            engine_max_pages: 100,
            compiled_patterns: vec![vec![]],
        };
        (ctx, stats)
    }

    /// 构造最小 SpiderResponse，仅 from_cache 字段可变。
    fn make_resp(from_cache: bool) -> SpiderResponse {
        SpiderResponse {
            url: "http://example.com/page".into(),
            status: 200,
            headers: HashMap::new(),
            body: vec![],
            request: SpiderRequest::get("http://example.com/page"),
            tracker: None,
            from_cache,
        }
    }

    /// 缓存命中（from_cache=true）时 stats.pages 不应递增。
    #[tokio::test]
    async fn process_response_from_cache_does_not_increment_pages() {
        let (ctx, stats) = make_ctx();
        let req = SpiderRequest::get("http://example.com/page");
        let resp = make_resp(true);
        process_response(&ctx, resp, &req, 0).await;
        assert_eq!(
            stats.pages.load(Ordering::SeqCst),
            0,
            "缓存命中时 pages 不应递增"
        );
    }

    /// 非缓存响应（from_cache=false）时 stats.pages 应递增。
    #[tokio::test]
    async fn process_response_not_from_cache_increments_pages() {
        let (ctx, stats) = make_ctx();
        let req = SpiderRequest::get("http://example.com/page");
        let resp = make_resp(false);
        process_response(&ctx, resp, &req, 0).await;
        assert_eq!(
            stats.pages.load(Ordering::SeqCst),
            1,
            "非缓存响应 pages 应递增到 1"
        );
    }
}
