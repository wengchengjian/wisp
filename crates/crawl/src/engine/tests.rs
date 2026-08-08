//! engine 单元测试与回归测试。

use super::*;
use crate::CrawlRequest;
use crate::engine::request::check_control_and_hook;
use crate::observability::events::{EventBus, Subscription};
use async_trait::async_trait;
use futures::StreamExt;
use std::collections::HashSet;
use std::time::Duration;

fn test_engine_config(fetch_mode: FetchMode, max_retries: u32) -> crate::engine::EngineConfig {
    let mut config = crate::engine::EngineConfig {
        fetch_mode,
        max_retries,
        max_concurrent: 8,
        obey_robots: false,
        max_pages: Some(100),
        max_refetch_rounds: 5,
        checkpoint_interval: 0,
        ..Default::default()
    };
    config.transport.http.timeout = Duration::from_millis(100);
    config.transport.max_concurrent_pages = 0;
    config
}

/// 最小 Spider：handle 返回空，不产出 items/follows，避免触碰事件通道。
struct DummySpider;

#[async_trait]
impl Spider for DummySpider {
    fn name(&self) -> &str {
        "dummy"
    }
    fn start_urls(&self) -> Vec<String> {
        vec![]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<CrawlRequest>) {
        (vec![], vec![])
    }
}

/// 可命名 Spider：默认接受所有 callback，用于验证同名 callback 歧义。
struct NamedSpider {
    name: String,
}

#[async_trait]
impl Spider for NamedSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        vec![]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<CrawlRequest>) {
        (vec![], vec![])
    }
}

/// handler 内 panic 不应击穿 process_response 工作线程。
struct PanicSpider;

#[async_trait]
impl Spider for PanicSpider {
    fn name(&self) -> &str {
        "panic"
    }
    fn start_urls(&self) -> Vec<String> {
        vec![]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<CrawlRequest>) {
        panic!("handler panic")
    }
}

/// 构造最小 EngineContext（单 Spider，Http 模式，无事件通道）。
fn make_ctx_with(
    config: crate::engine::EngineConfig,
    chain: middleware::MiddlewareChain,
    event_bus: EventBus,
) -> (EngineContext, Arc<SpiderStats>) {
    let stats = Arc::new(SpiderStats::new());
    let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<CrawlRequest>();
    let fetch_client = Arc::new(
        wisp_fetcher::FetchClient::new(config.transport.clone()).expect("build fetch client"),
    );
    let ctx = EngineContext {
        config,
        runtime: EngineRuntime {
            fetch_client,
            control: Arc::new(crate::control::EngineControl::new()),
            cache_store: None,
            checkpoint_store: None,
            last_checkpoint_at: Arc::new(tokio::sync::Mutex::new(None)),
            checkpoint_saving: Arc::new(AtomicBool::new(false)),
            autoscale: None,
            event_bus: Arc::new(event_bus),
            ua_middleware: None,
            custom_middlewares: Vec::new(),
            pipelines: Vec::new(),
        },
        state: EngineState {
            queue: QueueState {
                sched: Arc::new(scheduler::Scheduler::new()),
                follow_tx,
                follow_rx: Arc::new(Mutex::new(follow_rx)),
                work_notify: Arc::new(tokio::sync::Notify::new()),
            },
            middleware_chain: Arc::new(chain),
            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
            cf_locks: CfLockMap {
                locks: Arc::new(dashmap::DashMap::new()),
            },
            spiders: SpiderRegistry::new(
                vec![Arc::new(DummySpider) as Arc<dyn Spider>],
                vec![stats.clone()],
            ),
            run: RunState {
                abort_flag: Arc::new(AtomicBool::new(false)),
                pipeline_error: Arc::new(Mutex::new(None)),
                global_in_flight: Arc::new(AtomicUsize::new(0)),
                in_flight_requests: Arc::new(Mutex::new(HashMap::new())),
            },
        },
    };
    (ctx, stats)
}

/// 返回上下文与对应 stats 的 Arc 克隆，便于测试断言计数器。
fn make_ctx() -> (EngineContext, Arc<SpiderStats>) {
    make_ctx_with(
        test_engine_config(FetchMode::Http, 3),
        middleware::MiddlewareChain::new(),
        EventBus::new(),
    )
}

/// 构造最小 Response，仅 from_cache 字段可变。
fn make_resp(from_cache: bool) -> Response {
    Response::from_parts(wisp_core::ResponseParts {
        status: 200,
        url: "http://example.com/page".into(),
        headers: HashMap::new(),
        body: vec![],
        title: None,
        cookies: Vec::new(),
        request: CrawlRequest::get("http://example.com/page"),
        content_type: String::new(),
        from_cache,
    })
}

/// 缓存命中（from_cache=true）时 stats.pages 不应递增。
#[tokio::test]
async fn process_response_from_cache_does_not_increment_pages() {
    let (ctx, stats) = make_ctx();
    let resp = make_resp(true);
    process_response(&ctx, resp).await;
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
    let resp = make_resp(false);
    process_response(&ctx, resp).await;
    assert_eq!(
        stats.pages.load(Ordering::SeqCst),
        1,
        "非缓存响应 pages 应递增到 1"
    );
}

/// handler panic 时应被 process_response 隔离并计入错误，而不是向外传播。
#[tokio::test]
async fn process_response_isolates_handler_panic() {
    let (mut ctx, _stats) = make_ctx();
    ctx.state.spiders.router.spiders = vec![Arc::new(PanicSpider) as Arc<dyn Spider>];
    let resp = make_resp(false);
    process_response(&ctx, resp).await;
}

/// Task 3：验证 persist_spider_checkpoint 把 Scheduler 的 seen_urls 集合写入持久化 blob。
#[tokio::test]
async fn save_checkpoint_persists_seen_urls() {
    let (mut ctx, stats) = make_ctx();
    let store: Arc<dyn Store> = Arc::new(wisp_storage::MemoryStore::default());
    ctx.runtime.checkpoint_store = Some(store.clone());
    ctx.config.checkpoint_interval = 5;
    // push 两个 URL：进入 heap 与 seen 集合
    ctx.state
        .queue
        .sched
        .push(CrawlRequest::get("https://example.com/a"))
        .await;
    ctx.state
        .queue
        .sched
        .push(CrawlRequest::get("https://example.com/b"))
        .await;

    stats.pages.store(5, std::sync::atomic::Ordering::SeqCst);
    let spider = ctx.state.spiders.router.spiders[0].clone();
    maybe_persist_checkpoint(&ctx, &spider, &stats).await;

    // checkpoint 保存已在后台 spawn：轮询等待保存完成（防重入标志复位）
    let saving_flag = Arc::clone(&ctx.runtime.checkpoint_saving);
    for _ in 0..200 {
        if !saving_flag.load(std::sync::atomic::Ordering::SeqCst) {
            if store
                .load_checkpoint("dummy")
                .await
                .expect("load checkpoint ok")
                .is_some()
            {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let blob = store
        .load_checkpoint("dummy")
        .await
        .expect("load checkpoint ok")
        .expect("checkpoint should exist");
    let state: CrawlState = bincode::deserialize(&blob).expect("deserialize state");
    assert!(
        state.seen_urls.contains("https://example.com/a"),
        "seen_urls 必须包含已爬 URL a，当前 seen = {:?}",
        state.seen_urls
    );
    assert!(
        state.seen_urls.contains("https://example.com/b"),
        "seen_urls 必须包含已爬 URL b，当前 seen = {:?}",
        state.seen_urls
    );
}

/// 构造带 RetryMiddleware 的 EngineContext（max_retries 可配置）。
fn make_ctx_with_retry(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
    let mut chain = middleware::MiddlewareChain::new();
    chain
        .middlewares
        .push(Arc::new(middleware::builtin::RetryMiddleware::new(
            std::time::Duration::ZERO,
        )));
    make_ctx_with(
        test_engine_config(FetchMode::Http, max_retries),
        chain,
        EventBus::new(),
    )
}

/// ND-002-CORR 回归测试：fetch_with_retry 必须实际执行重试。
///
/// 原实现通过 `follow_tx → sched.push` 重新入队，被 `seen_exact` 去重静默丢弃，
/// 导致 RetryMiddleware 完全不工作。修复后 fetch_with_retry 在函数内同步循环重试，
/// retry_count 和 stats.retries 必须实际递增。
///
/// 使用不可达端口（127.0.0.1:1）触发 `connection refused`（即时返回，不依赖网络），
/// 该错误匹配 `is_retryable_network_error`（含 "connection refused"）。
#[tokio::test]
async fn fetch_with_retry_actually_retries_on_network_error() {
    // max_retries=2：应重试 2 次后失败
    let (ctx, stats) = make_ctx_with_retry(2);

    // 指向不可达端口触发 connection refused
    let req = CrawlRequest::get("http://127.0.0.1:1/");

    let (resp, err) = match fetch_with_retry(&ctx, &req).await {
        Ok(resp) => (Some(resp), None),
        Err(e) => (None, Some(e.to_string())),
    };

    // 重试耗尽后应返回错误（而非 Some(resp)）
    assert!(resp.is_none(), "重试耗尽后应返回 None 而非 Some(resp)");
    assert!(err.is_some(), "重试耗尽后应返回错误信息");
    let err_msg = err.unwrap();
    assert!(
        err_msg.contains("fetch failed")
            || err_msg.contains("Connection failed")
            || err_msg.contains("timed out"),
        "错误信息应含 'fetch failed'，实际: {}",
        err_msg
    );

    // 关键断言：stats.retries 必须等于 max_retries（2）
    // 原实现因 follow_tx 路径被去重，stats.retries 始终为 0
    assert_eq!(
        stats.retries.load(Ordering::SeqCst),
        2,
        "stats.retries 应等于 max_retries (2)，原 bug 导致此值为 0"
    );

    // errors 应递增 1（重试耗尽后计入一次错误）
    assert_eq!(
        stats.errors.load(Ordering::SeqCst),
        1,
        "重试耗尽后 errors 应为 1"
    );
}

/// ND-002-CORR 回归测试：max_retries=0 时不重试，直接失败。
#[tokio::test]
async fn fetch_with_retry_no_retry_when_max_retries_zero() {
    let (ctx, stats) = make_ctx_with_retry(0);

    let req = CrawlRequest::get("http://127.0.0.1:1/");
    let (resp, err) = match fetch_with_retry(&ctx, &req).await {
        Ok(resp) => (Some(resp), None),
        Err(e) => (None, Some(e.to_string())),
    };

    assert!(resp.is_none(), "应返回 None");
    assert!(err.is_some(), "应返回错误");

    assert_eq!(
        stats.retries.load(Ordering::SeqCst),
        0,
        "max_retries=0 时不应重试"
    );
    assert_eq!(stats.errors.load(Ordering::SeqCst), 1, "应计入一次错误");
}

/// 构造 Auto 模式 EngineContext（max_concurrent_pages=0 禁用浏览器池，
/// Stealth 模式快速返回 "browser pool not configured" 错误）。
fn make_ctx_auto(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
    let mut chain = middleware::MiddlewareChain::new();
    chain
        .middlewares
        .push(Arc::new(middleware::builtin::RetryMiddleware::new(
            std::time::Duration::ZERO,
        )));
    let mut config = test_engine_config(FetchMode::Auto, max_retries);
    config.transport.max_concurrent_pages = 0; // 禁用浏览器池，Stealth 模式快速失败
    make_ctx_with(config, chain, EventBus::new())
}

/// Auto 模式首次连接失败时，fetch_with_retry 应主动升级 Stealth 重试。
///
/// 连接层拦截（TLS reset/连接拒绝）无法被响应中间件 StealthUpgradeMiddleware
/// 检测（因为没有 HTTP 响应）。fetch_with_retry 在错误处理中主动升级：
/// 1. HTTP 模式失败 → learn(rule_engine, Stealth) + set override + continue
/// 2. Stealth 模式失败（此处 browser pool 未配置）→ 走正常错误流程
///
/// 验证：rule_engine 学到了该 URL 需要 Stealth（resolve 返回 Some）。
#[tokio::test]
async fn fetch_with_retry_auto_upgrades_to_stealth_on_first_failure() {
    let (ctx, _stats) = make_ctx_auto(0);

    let url = "http://127.0.0.1:1/auto-upgrade-test";
    let req = CrawlRequest::get(url);
    let (resp, err) = match fetch_with_retry(&ctx, &req).await {
        Ok(resp) => (Some(resp), None),
        Err(e) => (None, Some(e.to_string())),
    };

    // Stealth 模式也失败（browser pool 未配置），最终返回错误
    assert!(resp.is_none(), "Stealth 也失败时应返回 None");
    assert!(err.is_some(), "应返回错误信息");

    // 关键断言：rule_engine 应学到该 URL 需要 Stealth
    let resolved = ctx.state.rule_engine.lock().await.resolve(url);
    assert_eq!(
        resolved,
        Some(FetchMode::Stealth),
        "Auto 模式首次失败后 rule_engine 应学到 Stealth，实际: {:?}",
        resolved
    );
}

/// 已学习 Stealth 的 URL 不应重复触发 AutoFallback。
///
/// 场景：rule_engine 已学习某 URL 需要 Stealth（如之前请求失败 learn 过）。
/// 后续请求走 resolve 缓存直接用 Stealth，如果 Stealth 也失败（如无 Chrome），
/// 不应再次触发 AutoFallback（否则每个请求都打印升级日志）。
#[tokio::test]
async fn fetch_with_retry_no_duplicate_autofallback_for_learned_url() {
    let (ctx, _stats) = make_ctx_auto(0);

    let url = "http://127.0.0.1:1/learned-url";

    // 预先学习：模拟之前请求已 learn 过 Stealth
    {
        let mut rule_engine = ctx.state.rule_engine.lock().await;
        rule_engine.learn(url, FetchMode::Stealth);
        assert_eq!(rule_engine.auto_rule_count(), 1);
    }

    let req = CrawlRequest::get(url);
    let (resp, err) = match fetch_with_retry(&ctx, &req).await {
        Ok(resp) => (Some(resp), None),
        Err(e) => (None, Some(e.to_string())),
    };

    // Stealth 失败（browser pool 未配置），返回错误
    assert!(resp.is_none(), "Stealth 失败时应返回 None");
    assert!(err.is_some(), "应返回错误信息");

    // 关键断言：不应重复 learn（auto_rule_count 仍为 1）
    let rule_engine = ctx.state.rule_engine.lock().await;
    assert_eq!(
        rule_engine.auto_rule_count(),
        1,
        "已学习 Stealth 的 URL 不应重复触发 AutoFallback learn"
    );
}

// === ND-012-TEST：核心函数补充测试 ===
//
// 覆盖 check_control_and_hook 控制流、process_request 错误路径、
// sanitize_url 凭据脱敏、CrawlEvent 发送。

/// cancel(url) 后 check_control_and_hook 应返回 false（跳过请求）。
#[tokio::test]
async fn check_control_cancelled_url_returns_false() {
    let (ctx, _stats) = make_ctx();
    let url = "http://example.com/cancelled";
    ctx.runtime.control.cancel(url).await;
    let req = CrawlRequest::get(url);
    assert!(
        !check_control_and_hook(&ctx, &req, &ctx.state.spiders.router.spiders[0]).await,
        "cancelled URL 应返回 false"
    );
}

/// shutdown 后 check_control_and_hook 应返回 false。
#[tokio::test]
async fn check_control_shutdown_returns_false() {
    let (ctx, _stats) = make_ctx();
    ctx.runtime.control.shutdown();
    let req = CrawlRequest::get("http://example.com/any");
    assert!(
        !check_control_and_hook(&ctx, &req, &ctx.state.spiders.router.spiders[0]).await,
        "shutdown 后应返回 false"
    );
}

/// pause(url) + shutdown 应返回 false（不死锁，由 wait_if_paused 检测 shutdown 退出）。
#[tokio::test]
async fn check_control_pause_then_shutdown_returns_false() {
    let (ctx, _stats) = make_ctx();
    let url = "http://example.com/paused";
    // 先 pause，再 shutdown（避免 wait_if_paused 永久阻塞）
    ctx.runtime.control.pause(url).await;
    ctx.runtime.control.shutdown();
    let req = CrawlRequest::get(url);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        check_control_and_hook(&ctx, &req, &ctx.state.spiders.router.spiders[0]),
    )
    .await;
    assert!(result.is_ok(), "pause+shutdown 不应死锁");
    assert!(!result.unwrap(), "pause+shutdown 后应返回 false");
}

/// cancelled URL 时 process_request 应返回 None（不发起抓取）。
#[tokio::test]
async fn process_request_cancelled_url_returns_none() {
    let (ctx, stats) = make_ctx();
    let url = "http://example.com/cancelled";
    ctx.runtime.control.cancel(url).await;
    let req = CrawlRequest::get(url);
    let result = process_request(&ctx, req).await;
    assert!(result.is_none(), "cancelled URL 应返回 None");
    // pages 不应递增（未抓取）
    assert_eq!(stats.pages.load(Ordering::SeqCst), 0);
}

/// shutdown 时 process_request 应返回 None。
#[tokio::test]
async fn process_request_shutdown_returns_none() {
    let (ctx, _stats) = make_ctx();
    ctx.runtime.control.shutdown();
    let req = CrawlRequest::get("http://example.com/any");
    let result = process_request(&ctx, req).await;
    assert!(result.is_none(), "shutdown 时应返回 None");
}

/// 构造带订阅的 EngineContext，返回 (ctx, stats, subscription) 用于消费事件。
fn make_ctx_with_subscription(max_retries: u32) -> (EngineContext, Arc<SpiderStats>, Subscription) {
    let bus = EventBus::new();
    let sub = bus.subscribe(128);
    let mut chain = middleware::MiddlewareChain::new();
    chain
        .middlewares
        .push(Arc::new(middleware::builtin::RetryMiddleware::new(
            std::time::Duration::ZERO,
        )));
    let (ctx, stats) = make_ctx_with(test_engine_config(FetchMode::Http, max_retries), chain, bus);
    (ctx, stats, sub)
}

/// fetch 失败重试耗尽后应发送 CrawlEvent::Error。
#[tokio::test]
async fn process_request_emits_error_event_on_failure() {
    let (ctx, _stats, mut sub) = make_ctx_with_subscription(1);
    let req = CrawlRequest::get("http://127.0.0.1:1/");
    let result = process_request(&ctx, req).await;
    assert!(result.is_none(), "失败后应返回 None");

    // 应收到 Error 事件
    let mut got_error = false;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(500), sub.next()).await {
        if matches!(event, CrawlEvent::Error { .. }) {
            got_error = true;
            break;
        }
    }
    assert!(got_error, "应收到 CrawlEvent::Error 事件");
}

/// 重试期间应发送 CrawlEvent::Retry 事件（max_retries=2 时至少 1 次 Retry）。
#[tokio::test]
async fn process_request_emits_retry_events() {
    let (ctx, _stats, mut sub) = make_ctx_with_subscription(2);
    let req = CrawlRequest::get("http://127.0.0.1:1/");
    let _ = process_request(&ctx, req).await;

    // 收集所有事件
    let mut retry_count = 0;
    let mut got_error = false;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(500), sub.next()).await {
        match event {
            CrawlEvent::Retry { attempt, max, .. } => {
                assert_eq!(max, 2, "Retry 事件 max 应为 2");
                assert!(attempt >= 1, "attempt 应 >= 1");
                retry_count += 1;
            }
            CrawlEvent::Error { .. } => {
                got_error = true;
                break;
            }
            _ => {}
        }
    }
    assert!(retry_count >= 1, "应至少发送 1 次 Retry 事件");
    assert!(got_error, "重试耗尽后应发送 Error 事件");
}

/// record_status：状态码计数应正确累加。
#[test]
fn record_status_increments_counter() {
    let stats = Arc::new(SpiderStats::new());
    stats.record_status(200);
    stats.record_status(200);
    stats.record_status(404);
    let snapshot = stats.status_codes_snapshot();
    assert_eq!(snapshot.get(&200).copied(), Some(2));
    assert_eq!(snapshot.get(&404).copied(), Some(1));
}

/// SpiderStats::snapshot：应正确填充统计快照字段。
#[test]
fn snapshot_populates_fields() {
    let stats = Arc::new(SpiderStats::new());
    stats.pages.store(10, Ordering::SeqCst);
    stats.items.store(50, Ordering::SeqCst);
    stats.errors.store(2, Ordering::SeqCst);
    stats.retries.store(5, Ordering::SeqCst);
    let snapshot = stats.snapshot();
    assert_eq!(snapshot.pages_crawled, 10);
    assert_eq!(snapshot.items_scraped, 50);
    assert_eq!(snapshot.errors, 2);
    assert_eq!(snapshot.retry_count, 5);
}

/// 同名 callback 被多个 Spider 接受时，未绑定请求应视为歧义，不静默路由到第一个 Spider。
#[test]
fn ambiguous_callback_does_not_route_to_first_spider() {
    let (mut ctx, _stats) = make_ctx();
    ctx.state.spiders.router.spiders = vec![
        Arc::new(NamedSpider { name: "a".into() }) as Arc<dyn Spider>,
        Arc::new(NamedSpider { name: "b".into() }) as Arc<dyn Spider>,
    ];
    ctx.state.spiders.all_stats = vec![Arc::new(SpiderStats::new()), Arc::new(SpiderStats::new())];
    let req = CrawlRequest::get("http://example.com/detail").with_callback("detail");
    assert_eq!(
        ctx.state.spiders.spider_index_for(&req),
        None,
        "同名 callback 多 Spider 应视为歧义，而不是选第一个"
    );
    let req = req.with_spider("b");
    assert_eq!(
        ctx.state.spiders.spider_index_for(&req),
        Some(1),
        "显式 spider 绑定应覆盖歧义"
    );
}

/// 域名受限 Spider：仅允许 allowed.example，用于验证 offsite 计数。
struct DomainRestrictedSpider;

#[async_trait]
impl Spider for DomainRestrictedSpider {
    fn name(&self) -> &str {
        "domain-restricted"
    }
    fn start_urls(&self) -> Vec<String> {
        vec![]
    }
    async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<CrawlRequest>) {
        (vec![], vec![])
    }
    fn allowed_domains(&self) -> HashSet<String> {
        HashSet::from(["allowed.example".to_string()])
    }
}

#[tokio::test]
async fn offsite_request_increments_offsite_counter() {
    let (mut ctx, stats) = make_ctx();
    ctx.state.spiders.router.spiders = vec![Arc::new(DomainRestrictedSpider) as Arc<dyn Spider>];
    let req = CrawlRequest::get("https://blocked.example.com/");
    assert!(process_request(&ctx, req).await.is_none());
    assert_eq!(stats.offsite.load(Ordering::SeqCst), 1);
}
