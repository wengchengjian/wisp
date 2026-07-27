# Review package: 88e7769..8859255

## Commits
8859255 refactor: 删除 EngineShared，字段合并到 EngineContext 顶层

## Files changed
 src/crawl/engine.rs | 178 ++++++++++++++++++++++++++--------------------------
 src/crawl/runner.rs |  94 +++++++++++++--------------
 2 files changed, 134 insertions(+), 138 deletions(-)

## Diff
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index bb67f9f..e5f238e 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -24,78 +24,76 @@ use super::stats::SpiderStats;
 use super::{
     auto, control, middleware, scheduler, CrawlEvent, CrawlState, CrawlStats, Request, Response,
     Spider,
 };
 use crate::error::Result;
 use crate::fetcher::FetchMode;
 use crate::http::Client;
 
 // === EngineContext: 打包所有共享状态 ===
 
-/// Engine 运行时上下文（单 Spider），由三层组成。
+/// Engine 运行时上下文（单 Spider）。
 ///
-/// - `config`: 只读配置（Arc 共享，构建后不可变）
-/// - `client`: 共享 FetchClient（跨 task 复用）
-/// - `shared`: 跨 task 共享的可变状态
-/// - `state`: per-run 可变状态
+/// PR4 重构：删除 EngineShared 子结构，字段直接展开到 EngineContext 顶层。
+/// 分为三组：
+/// - 只读配置：config (Arc<runner::EngineConfig>) + client (Arc<FetchClient>)
+/// - 跨 task 共享可变状态：sched / follow_tx / proxy_clients / control / work_notify / middleware_chain / rule_engine / cf_domain_locks
+/// - per-run 可变状态：state
 pub(crate) struct EngineContext {
+    // === 只读配置 ===
     pub config: Arc<crate::crawl::runner::EngineConfig>,
     pub client: Arc<crate::fetcher::FetchClient>,
-    pub shared: EngineShared,
-    pub state: EngineState,
-}
 
-/// 跨 task 共享的可变状态。
-pub(crate) struct EngineShared {
+    // === 跨 task 共享可变状态（原 EngineShared） ===
     pub sched: Arc<scheduler::Scheduler>,
     pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
-    // OPTIMIZE: follow_rx 移出 EngineShared，由 runner.rs unfold 状态持有。
-    // 旧实现 `Arc<Mutex<UnboundedReceiver>>` 串行化所有 follow drain，但 Receiver 是
-    // 单消费者类型，无需 Mutex；保留 Mutex 只增加锁争用与无谓 await 开销。
     /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）。
     ///
     /// ND-009-SEC：使用 moka::sync::Cache 限制最大条目数，防止攻击者注入大量
     /// 不同 proxy URL 导致无界增长 OOM。默认上限 1024，超限时 LRU 淘汰。
     pub proxy_clients: Arc<moka::sync::Cache<String, Arc<Client>>>,
     pub control: Arc<control::EngineControl>,
     pub work_notify: Arc<tokio::sync::Notify>,
     pub middleware_chain: Arc<middleware::MiddlewareChain>,
     pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
     /// 域名级 CF 挑战锁：防止初始并发请求全部走浏览器。
     /// 第一个请求获取锁并解决 CF，其他请求等待后复用 cookie。
     /// 用 moka::Cache（max 1024，LRU 淘汰）避免无界增长。
     pub cf_domain_locks: Arc<moka::sync::Cache<String, Arc<tokio::sync::Mutex<()>>>>,
+
+    // === per-run 可变状态 ===
+    pub state: EngineState,
 }
 
 /// per-run 可变状态。
 pub(crate) struct EngineState {
     pub spider: Arc<dyn Spider>,
     pub stats: Arc<SpiderStats>,
     pub items: Arc<Mutex<Vec<Value>>>,
     pub abort_flag: Arc<AtomicBool>,
     pub start: std::time::Instant,
     pub tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
     pub global_in_flight: Arc<AtomicUsize>,
 }
 
 // === 核心函数：处理单个请求 ===
 
 /// Stage 1: 控制状态检查 + Spider 钩子（基础设施级，不可中间件化）。
 async fn check_control_and_hook(ctx: &EngineContext, req: &Request) -> bool {
     // per-Engine 控制状态检查
-    if ctx.shared.control.is_cancelled(&req.url).await {
+    if ctx.control.is_cancelled(&req.url).await {
         return false;
     }
-    if !ctx.shared.control.wait_if_paused(&req.url).await {
+    if !ctx.control.wait_if_paused(&req.url).await {
         return false;
     }
-    if ctx.shared.control.is_shutdown() {
+    if ctx.control.is_shutdown() {
         return false;
     }
     // Spider 异步钩子
     match ctx.state.spider.on_before_request(req).await {
         super::RequestAction::Proceed => true,
         super::RequestAction::Skip => false,
         super::RequestAction::Delay(d) => {
             tokio::time::sleep(d).await;
             true
         }
@@ -118,24 +116,23 @@ async fn check_control_and_hook(ctx: &EngineContext, req: &Request) -> bool {
 #[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
 pub(crate) async fn process_request(ctx: &EngineContext, req: Request) -> Option<Response> {
     // 1. 控制状态 + Spider 钩子
     if !check_control_and_hook(ctx, &req).await {
         return None;
     }
 
     // 2. 中间件请求链（所有策略过滤在此发生）
     let mut req = req;
     let stats = &ctx.state.stats;
-    if !ctx.shared.middleware_chain.is_empty() {
+    if !ctx.middleware_chain.is_empty() {
         let crawl_ctx = build_crawl_context(ctx);
         match ctx
-            .shared
             .middleware_chain
             .run_request_middlewares(&mut req, &crawl_ctx)
             .await
         {
             middleware::MwAction::Skip => return None,
             middleware::MwAction::Abort(reason) => {
                 tracing::warn!("middleware abort: {} - {}", reason, sanitize_url(&req.url));
                 return None;
             }
             middleware::MwAction::Respond(cached_resp) => {
@@ -179,25 +176,24 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
     let stats = &ctx.state.stats;
 
     if !resp.from_cache {
         stats.pages.fetch_add(1, Ordering::SeqCst);
     }
     let page_url = resp.url.clone();
 
     // 中间件链：响应后拦截（支持 Refetch 循环，最多 5 轮）
     let mut resp = resp;
     let mut refetch_depth = 0u32;
-    if !ctx.shared.middleware_chain.is_empty() {
+    if !ctx.middleware_chain.is_empty() {
         loop {
             let crawl_ctx = build_crawl_context(ctx);
             match ctx
-                .shared
                 .middleware_chain
                 .run_response_middlewares(&mut resp, &crawl_ctx)
                 .await
             {
                 middleware::MwAction::Skip => return,
                 middleware::MwAction::Abort(reason) => {
                     tracing::warn!(
                         "response middleware abort: {} - {}",
                         reason,
                         sanitize_url(&page_url)
@@ -267,39 +263,38 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
             sanitize_url(&page_url_for_log),
             items.len(),
             follows.len()
         );
     }
 
     // 发送 items：经过 pipeline 链处理后收集到 ctx.items 和 tx（若有）
     // ND-009-PERF：crawl_ctx 在循环外构建一次，避免每 item 重建
     // OPTIMIZE: 本地 Vec 收集所有 processed items，单次 lock 批量 push，
     // 避免 N 次 lock().await.push()。tx 改 try_send 非阻塞，消费者慢时不卡核心路径。
-    let pipeline_crawl_ctx = if ctx.shared.middleware_chain.is_empty() {
+    let pipeline_crawl_ctx = if ctx.middleware_chain.is_empty() {
         None
     } else {
         Some(build_crawl_context(ctx))
     };
 
     // OPTIMIZE: 预分配容量，避免 Vec 增长时多次 realloc。
     let mut processed_items: Vec<Value> = Vec::with_capacity(items.len());
 
     for item in items {
         // 先经过 Spider 的 on_item 钩子
         let item = match spider.on_item(item).await {
             Some(i) => i,
             None => continue,
         };
         // 再经过中间件 pipeline 链
         let item = if let Some(ref crawl_ctx) = pipeline_crawl_ctx {
-            ctx.shared
-                .middleware_chain
+            ctx.middleware_chain
                 .run_pipelines(item, crawl_ctx)
                 .await
         } else {
             Some(item)
         };
         if let Some(processed) = item {
             stats.items.fetch_add(1, Ordering::SeqCst);
             // OPTIMIZE: 事件用 try_send 非阻塞。channel 满时丢失事件优于阻塞核心路径。
             if let Some(ref tx) = ctx.state.tx {
                 if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
@@ -311,26 +306,26 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
             processed_items.push(processed);
         }
     }
 
     // OPTIMIZE: 单次 lock 批量 push，锁争用从 N 次降为 1 次。
     if !processed_items.is_empty() {
         let mut items_lock = ctx.state.items.lock().await;
         items_lock.extend(processed_items);
     }
     for f in follows {
-        if ctx.shared.follow_tx.send(f).is_err() {
+        if ctx.follow_tx.send(f).is_err() {
             tracing::debug!("follow_tx closed, dropping follow request");
         }
     }
     // 通知主循环有新工作到来
-    ctx.shared.work_notify.notify_one();
+    ctx.work_notify.notify_one();
 
     // PageScraped 事件
     if let Some(ref tx) = ctx.state.tx {
         let status_codes_snapshot = stats.status_codes_snapshot();
         let _ = tx
             .send(CrawlEvent::PageScraped {
                 url: sanitize_url(&page_url), // ND-007-SEC：事件中脱敏 URL 凭据
                 stats: snapshot_stats_for(stats, status_codes_snapshot, ctx.state.start),
             })
             .await;
@@ -359,23 +354,23 @@ async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>
     // 同步重试循环：clone 一次，循环内修改 retry_count
     let mut current_req = req.clone();
 
     loop {
         let proxy = current_req.proxy.clone();
         match fetch_page(
             &ctx.client,
             &current_req,
             proxy.as_deref(),
             ctx.config.fetch_mode,
-            &ctx.shared.rule_engine,
-            &ctx.shared.proxy_clients,
-            &ctx.shared.cf_domain_locks,
+            &ctx.rule_engine,
+            &ctx.proxy_clients,
+            &ctx.cf_domain_locks,
         )
         .await
         {
             Ok(resp) => {
                 record_status(stats, resp.status);
                 if ctx.state.spider.is_blocked(&resp) {
                     stats.blocked.fetch_add(1, Ordering::SeqCst);
                 }
                 return (Some(resp), None);
             }
@@ -386,46 +381,45 @@ async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>
                 //
                 // 关键：只对 rule_engine 未学习的 URL 触发升级。已学习 Stealth 的 URL
                 // 走 resolve 缓存直接用 Stealth，此时失败是 Stealth 本身的问题（如无 Chrome），
                 // 不应重复"升级"（否则每个请求都会打印 AutoFallback 日志）。
                 if ctx.config.fetch_mode == FetchMode::Auto
                     && current_req.fetch_mode_override.is_none()
                     && current_req.retry_count == 0
                 {
                     // OPTIMIZE: 合并 resolve+learn 到单次锁，避免两次锁争用。
                     let should_upgrade = {
-                        let mut engine = ctx.shared.rule_engine.lock().await;
+                        let mut engine = ctx.rule_engine.lock().await;
                         if engine.resolve(&current_req.url) == Some(FetchMode::Stealth) {
                             false
                         } else {
                             engine.learn(&current_req.url, FetchMode::Stealth);
                             true
                         }
                     };
                     if should_upgrade {
                         tracing::info!(
                             "AutoFallback: '{}' 首次抓取失败 ({}), 升级 Stealth 重试",
                             sanitize_url(&current_req.url),
                             e
                         );
                         current_req.fetch_mode_override = Some(FetchMode::Stealth);
                         continue;
                     }
                 }
 
                 // 调用错误中间件：只决定"是否重试"，不维护计数
-                let action = if ctx.shared.middleware_chain.is_empty() {
+                let action = if ctx.middleware_chain.is_empty() {
                     middleware::ErrorAction::Propagate
                 } else {
                     let crawl_ctx = build_crawl_context(ctx);
-                    ctx.shared
-                        .middleware_chain
+                    ctx.middleware_chain
                         .run_error_middlewares(&current_req, &e.to_string(), &crawl_ctx)
                         .await
                 };
 
                 match action {
                     middleware::ErrorAction::Retry if current_req.retry_count < max_retries => {
                         current_req.retry_count += 1;
                         stats.retries.fetch_add(1, Ordering::SeqCst);
 
                         // OPTIMIZE: 指数退避 + 抖动，避免失败场景下的 Thundering Herd。
@@ -949,30 +943,28 @@ mod tests {
                 obey_robots: false,
                 max_pages: 100,
                 max_retries: 3,
                 max_refetch_rounds: 5,
                 ..Default::default()
             }),
             client: Arc::new(
                 crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                     .expect("build fetch client"),
             ),
-            shared: EngineShared {
-                sched: Arc::new(scheduler::Scheduler::new()),
-                follow_tx,
-                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-                control: Arc::new(control::EngineControl::new()),
-                work_notify: Arc::new(tokio::sync::Notify::new()),
-                middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
-                rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
-                cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-            },
+            sched: Arc::new(scheduler::Scheduler::new()),
+            follow_tx,
+            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
+            control: Arc::new(control::EngineControl::new()),
+            work_notify: Arc::new(tokio::sync::Notify::new()),
+            middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
+            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
+            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
                 items: Arc::new(Mutex::new(Vec::new())),
                 abort_flag: Arc::new(AtomicBool::new(false)),
                 start: Instant::now(),
                 tx: None,
                 global_in_flight: Arc::new(AtomicUsize::new(0)),
             },
         };
@@ -1069,30 +1061,28 @@ mod tests {
                 obey_robots: false,
                 max_pages: 100,
                 max_retries,
                 max_refetch_rounds: 5,
                 ..Default::default()
             }),
             client: Arc::new(
                 crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                     .expect("build fetch client"),
             ),
-            shared: EngineShared {
-                sched: Arc::new(scheduler::Scheduler::new()),
-                follow_tx,
-                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-                control: Arc::new(control::EngineControl::new()),
-                work_notify: Arc::new(tokio::sync::Notify::new()),
-                middleware_chain: Arc::new(chain),
-                rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
-                cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-            },
+            sched: Arc::new(scheduler::Scheduler::new()),
+            follow_tx,
+            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
+            control: Arc::new(control::EngineControl::new()),
+            work_notify: Arc::new(tokio::sync::Notify::new()),
+            middleware_chain: Arc::new(chain),
+            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
+            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
                 items: Arc::new(Mutex::new(Vec::new())),
                 abort_flag: Arc::new(AtomicBool::new(false)),
                 start: Instant::now(),
                 tx: None,
                 global_in_flight: Arc::new(AtomicUsize::new(0)),
             },
         };
@@ -1183,30 +1173,28 @@ mod tests {
                 max_refetch_rounds: 5,
                 ..Default::default()
             }),
             client: {
                 let fetch_config = crate::fetcher::FetchClientConfig {
                     max_concurrent_pages: 0, // 禁用浏览器池，Stealth 模式快速失败
                     ..Default::default()
                 };
                 Arc::new(crate::fetcher::FetchClient::new(fetch_config).expect("build fetch client"))
             },
-            shared: EngineShared {
-                sched: Arc::new(scheduler::Scheduler::new()),
-                follow_tx,
-                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-                control: Arc::new(control::EngineControl::new()),
-                work_notify: Arc::new(tokio::sync::Notify::new()),
-                middleware_chain: Arc::new(chain),
-                rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
-                cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-            },
+            sched: Arc::new(scheduler::Scheduler::new()),
+            follow_tx,
+            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
+            control: Arc::new(control::EngineControl::new()),
+            work_notify: Arc::new(tokio::sync::Notify::new()),
+            middleware_chain: Arc::new(chain),
+            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
+            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
                 items: Arc::new(Mutex::new(Vec::new())),
                 abort_flag: Arc::new(AtomicBool::new(false)),
                 start: Instant::now(),
                 tx: None,
                 global_in_flight: Arc::new(AtomicUsize::new(0)),
             },
         };
@@ -1227,55 +1215,55 @@ mod tests {
 
         let url = "http://127.0.0.1:1/auto-upgrade-test";
         let req = Request::get(url);
         let (resp, err) = fetch_dispatch(&ctx, &req).await;
 
         // Stealth 模式也失败（browser pool 未配置），最终返回错误
         assert!(resp.is_none(), "Stealth 也失败时应返回 None");
         assert!(err.is_some(), "应返回错误信息");
 
         // 关键断言：rule_engine 应学到该 URL 需要 Stealth
-        let resolved = ctx.shared.rule_engine.lock().await.resolve(url);
+        let resolved = ctx.rule_engine.lock().await.resolve(url);
         assert_eq!(
             resolved,
             Some(FetchMode::Stealth),
             "Auto 模式首次失败后 rule_engine 应学到 Stealth，实际: {resolved:?}"
         );
     }
 
     /// 已学习 Stealth 的 URL 不应重复触发 AutoFallback。
     ///
     /// 场景：rule_engine 已学习某 URL 需要 Stealth（如之前请求失败 learn 过）。
     /// 后续请求走 resolve 缓存直接用 Stealth，如果 Stealth 也失败（如无 Chrome），
     /// 不应再次触发 AutoFallback（否则每个请求都打印升级日志）。
     #[tokio::test]
     async fn fetch_dispatch_no_duplicate_autofallback_for_learned_url() {
         let (ctx, _stats) = make_ctx_auto(0);
 
         let url = "http://127.0.0.1:1/learned-url";
 
         // 预先学习：模拟之前请求已 learn 过 Stealth
         {
-            let mut rule_engine = ctx.shared.rule_engine.lock().await;
+            let mut rule_engine = ctx.rule_engine.lock().await;
             rule_engine.learn(url, FetchMode::Stealth);
             assert_eq!(rule_engine.auto_rule_count(), 1);
         }
 
         let req = Request::get(url);
         let (resp, err) = fetch_dispatch(&ctx, &req).await;
 
         // Stealth 失败（browser pool 未配置），返回错误
         assert!(resp.is_none(), "Stealth 失败时应返回 None");
         assert!(err.is_some(), "应返回错误信息");
 
         // 关键断言：不应重复 learn（auto_rule_count 仍为 1）
-        let rule_engine = ctx.shared.rule_engine.lock().await;
+        let rule_engine = ctx.rule_engine.lock().await;
         assert_eq!(
             rule_engine.auto_rule_count(),
             1,
             "已学习 Stealth 的 URL 不应重复触发 AutoFallback learn"
         );
     }
 
     // === Task 4 测试：指数退避 + 抖动 + 单次锁 ===
 
     /// 指数退避算法：base * 2^attempt，封顶 30s。
@@ -1341,76 +1329,76 @@ mod tests {
     // === ND-012-TEST：核心函数补充测试 ===
     //
     // 覆盖 check_control_and_hook 控制流、process_request 错误路径、
     // sanitize_url 凭据脱敏、CrawlEvent 发送。
 
     /// cancel(url) 后 check_control_and_hook 应返回 false（跳过请求）。
     #[tokio::test]
     async fn check_control_cancelled_url_returns_false() {
         let (ctx, _stats) = make_ctx();
         let url = "http://example.com/cancelled";
-        ctx.shared.control.cancel(url).await;
+        ctx.control.cancel(url).await;
         let req = Request::get(url);
         assert!(
             !check_control_and_hook(&ctx, &req).await,
             "cancelled URL 应返回 false"
         );
     }
 
     /// shutdown 后 check_control_and_hook 应返回 false。
     #[tokio::test]
     async fn check_control_shutdown_returns_false() {
         let (ctx, _stats) = make_ctx();
-        ctx.shared.control.shutdown();
+        ctx.control.shutdown();
         let req = Request::get("http://example.com/any");
         assert!(
             !check_control_and_hook(&ctx, &req).await,
             "shutdown 后应返回 false"
         );
     }
 
     /// pause(url) + shutdown 应返回 false（不死锁，由 wait_if_paused 检测 shutdown 退出）。
     #[tokio::test]
     async fn check_control_pause_then_shutdown_returns_false() {
         let (ctx, _stats) = make_ctx();
         let url = "http://example.com/paused";
         // 先 pause，再 shutdown（避免 wait_if_paused 永久阻塞）
-        ctx.shared.control.pause(url).await;
-        ctx.shared.control.shutdown();
+        ctx.control.pause(url).await;
+        ctx.control.shutdown();
         let req = Request::get(url);
         let result = tokio::time::timeout(
             std::time::Duration::from_secs(2),
             check_control_and_hook(&ctx, &req),
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
-        ctx.shared.control.cancel(url).await;
+        ctx.control.cancel(url).await;
         let req = Request::get(url);
         let result = process_request(&ctx, req).await;
         assert!(result.is_none(), "cancelled URL 应返回 None");
         // pages 不应递增（未抓取）
         assert_eq!(stats.pages.load(Ordering::SeqCst), 0);
     }
 
     /// shutdown 时 process_request 应返回 None。
     #[tokio::test]
     async fn process_request_shutdown_returns_none() {
         let (ctx, _stats) = make_ctx();
-        ctx.shared.control.shutdown();
+        ctx.control.shutdown();
         let req = Request::get("http://example.com/any");
         let result = process_request(&ctx, req).await;
         assert!(result.is_none(), "shutdown 时应返回 None");
     }
 
     /// 构造带事件通道的 EngineContext，返回 (ctx, stats, rx) 用于消费事件。
     fn make_ctx_with_tx(
         max_retries: u32,
     ) -> (
         EngineContext,
@@ -1427,38 +1415,36 @@ mod tests {
                 obey_robots: false,
                 max_pages: 100,
                 max_retries,
                 max_refetch_rounds: 5,
                 ..Default::default()
             }),
             client: Arc::new(
                 crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                     .expect("build fetch client"),
             ),
-            shared: EngineShared {
-                sched: Arc::new(scheduler::Scheduler::new()),
-                follow_tx,
-                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-                control: Arc::new(control::EngineControl::new()),
-                work_notify: Arc::new(tokio::sync::Notify::new()),
-                middleware_chain: Arc::new({
-                    let mut chain = middleware::MiddlewareChain::new();
-                    chain
-                        .middlewares
-                        .push(Arc::new(middleware::builtin::RetryMiddleware::new(
-                            max_retries,
-                        )));
-                    chain
-                }),
-                rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
-                cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-            },
+            sched: Arc::new(scheduler::Scheduler::new()),
+            follow_tx,
+            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
+            control: Arc::new(control::EngineControl::new()),
+            work_notify: Arc::new(tokio::sync::Notify::new()),
+            middleware_chain: Arc::new({
+                let mut chain = middleware::MiddlewareChain::new();
+                chain
+                    .middlewares
+                    .push(Arc::new(middleware::builtin::RetryMiddleware::new(
+                        max_retries,
+                    )));
+                chain
+            }),
+            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
+            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
                 items: Arc::new(Mutex::new(Vec::new())),
                 abort_flag: Arc::new(AtomicBool::new(false)),
                 start: Instant::now(),
                 tx: Some(tx),
                 global_in_flight: Arc::new(AtomicUsize::new(0)),
             },
         };
@@ -1639,11 +1625,25 @@ mod tests {
     #[test]
     fn test_engine_context_config_is_arc_runner_config() {
         let (ctx, _stats) = make_ctx();
         // 验证 config 字段类型为 Arc<crate::crawl::runner::EngineConfig>
         let _config: &crate::crawl::runner::EngineConfig = &ctx.config;
         // 验证 client 字段独立持有（不再嵌套在 config 中）
         let _client: &Arc<crate::fetcher::FetchClient> = &ctx.client;
         assert_eq!(ctx.config.fetch_mode, crate::fetcher::FetchMode::Http);
         assert_eq!(ctx.config.max_concurrent, 8);
     }
+
+    /// PR4 Task 5：验证 EngineContext 直接持有 sched/control 等字段（无 shared 子结构）。
+    #[test]
+    fn test_engine_context_no_shared_substruct() {
+        let (ctx, _stats) = make_ctx();
+        // 直接访问 ctx.sched（原 ctx.shared.sched）
+        let _sched: &Arc<scheduler::Scheduler> = &ctx.sched;
+        // 直接访问 ctx.control
+        let _control: &Arc<control::EngineControl> = &ctx.control;
+        // 直接访问 ctx.work_notify
+        let _notify: &Arc<tokio::sync::Notify> = &ctx.work_notify;
+        // 直接访问 ctx.middleware_chain
+        let _chain: &Arc<middleware::MiddlewareChain> = &ctx.middleware_chain;
+    }
 }
diff --git a/src/crawl/runner.rs b/src/crawl/runner.rs
index ccce301..598838d 100644
--- a/src/crawl/runner.rs
+++ b/src/crawl/runner.rs
@@ -293,80 +293,77 @@ impl Engine {
 
         spider.on_start().await;
 
         // 默认中间件注入所需资源（在 ctx 字面量 move 这些 Arc 前提取）
         let mw_http_client = fetch_client.http_arc();
         let mw_robots_cache = robots_cache.clone();
 
         let ctx = Arc::new(engine::EngineContext {
             config: Arc::clone(self.config()),
             client: fetch_client,
-            shared: engine::EngineShared {
-                sched: sched.clone(),
-                follow_tx,
-                // ND-009-SEC：moka::Cache 限制 proxy client 缓存最大 1024 条
-                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
-                control: self.control.clone(),
-                work_notify: Arc::new(tokio::sync::Notify::new()),
-                middleware_chain: {
-                    // 默认中间件链：按 fetch_mode + spider 配置注入（详见 builtin::default_middlewares）
-                    let defaults = middleware::builtin::default_middlewares(
-                        middleware::builtin::DefaultMiddlewareConfig {
-                            fetch_mode,
-                            delay: self.config().download_delay,
-                            obey_robots,
-                            allowed_domains: spider.allowed_domains(),
-                            max_depth,
-                            cache_store: self.cache_store.clone(),
-                            http_client: mw_http_client,
-                            robots_cache: mw_robots_cache,
-                            rule_engine: rule_engine.clone(),
-                            max_retries: self.config().max_retries,
-                        },
-                    );
-                    let mut chain = middleware::MiddlewareChain::new();
-                    // 用户中间件 + 默认中间件合并，sort 按 priority 统一排序
-                    chain.middlewares = spider.middlewares();
-                    chain.middlewares.extend(defaults);
-                    chain.pipelines = spider.pipelines();
-                    chain.sort();
-                    Arc::new(chain)
-                },
-                rule_engine,
-                cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
+            sched: sched.clone(),
+            follow_tx,
+            // ND-009-SEC：moka::Cache 限制 proxy client 缓存最大 1024 条
+            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
+            control: self.control.clone(),
+            work_notify: Arc::new(tokio::sync::Notify::new()),
+            middleware_chain: {
+                // 默认中间件链：按 fetch_mode + spider 配置注入（详见 builtin::default_middlewares）
+                let defaults = middleware::builtin::default_middlewares(
+                    middleware::builtin::DefaultMiddlewareConfig {
+                        fetch_mode,
+                        delay: self.config().download_delay,
+                        obey_robots,
+                        allowed_domains: spider.allowed_domains(),
+                        max_depth,
+                        cache_store: self.cache_store.clone(),
+                        http_client: mw_http_client,
+                        robots_cache: mw_robots_cache,
+                        rule_engine: rule_engine.clone(),
+                        max_retries: self.config().max_retries,
+                    },
+                );
+                let mut chain = middleware::MiddlewareChain::new();
+                // 用户中间件 + 默认中间件合并，sort 按 priority 统一排序
+                chain.middlewares = spider.middlewares();
+                chain.middlewares.extend(defaults);
+                chain.pipelines = spider.pipelines();
+                chain.sort();
+                Arc::new(chain)
             },
+            rule_engine,
+            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
             state: engine::EngineState {
                 spider: spider.clone(),
                 stats: stats.clone(),
                 items,
                 abort_flag: Arc::new(AtomicBool::new(false)),
                 start: std::time::Instant::now(),
                 tx,
                 global_in_flight: Arc::new(AtomicUsize::new(0)),
             },
         });
 
         // 中间件初始化：在爬取开始前调用所有中间件的 init + pipeline 的 open
-        if !ctx.shared.middleware_chain.is_empty() {
+        if !ctx.middleware_chain.is_empty() {
             let crawl_ctx = engine::build_crawl_context(&ctx);
-            ctx.shared.middleware_chain.run_init(&crawl_ctx).await;
-            ctx.shared
-                .middleware_chain
+            ctx.middleware_chain.run_init(&crawl_ctx).await;
+            ctx.middleware_chain
                 .run_pipelines_open(&crawl_ctx)
                 .await;
         }
 
         // 启用 autoscale 时，spawn 后台 autoscaler task
         let autoscaler_handle = if let Some(ref pool) = self.autoscale {
             // ND-004-CORR：注入 work_notify，autoscale 扩容时唤醒主循环，
             // 避免主循环 10ms timeout 轮询。
-            pool.set_work_notify(Arc::clone(&ctx.shared.work_notify));
+            pool.set_work_notify(Arc::clone(&ctx.work_notify));
             let pool = Arc::clone(pool);
             let stats = Arc::clone(&stats);
             Some(tokio::spawn(async move {
                 pool.run_autoscaler(stats).await;
             }))
         } else {
             None
         };
 
         // 构建并发流：单 Spider，无路由
@@ -378,46 +375,46 @@ impl Engine {
                 pool.max_concurrency()
             } else {
                 ctx.config.max_concurrent
             };
             // OPTIMIZE: follow_rx move 进 unfold 状态。UnboundedReceiver 是单消费者，
             // 无需 Mutex 串行化；旧实现 `Arc<Mutex<UnboundedReceiver>>` 的锁是冗余的。
             stream::unfold((ctx, follow_rx), move |(ctx, mut rx)| {
                 let autoscale = autoscale.clone();
                 async move {
                     loop {
-                        if ctx.shared.control.is_shutdown()
+                        if ctx.control.is_shutdown()
                             || ctx.state.abort_flag.load(Ordering::SeqCst)
                         {
                             return None;
                         }
 
                         // OPTIMIZE: 直接 try_recv drain follow channel，无 Mutex 锁争用。
                         // Receiver 是单消费者类型，串行化访问无意义。
                         while let Ok(req) = rx.try_recv() {
-                            ctx.shared.sched.push(req).await;
+                            ctx.sched.push(req).await;
                         }
 
                         // 引擎级 max_pages 兜底
                         let pages = ctx.state.stats.pages.load(Ordering::SeqCst);
                         if pages + ctx.state.global_in_flight.load(Ordering::SeqCst)
                             >= ctx.config.max_pages
                         {
                             if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                 return None;
                             }
                             tokio::task::yield_now().await;
                             continue;
                         }
 
                         // Spider until 终止条件检查
-                        let queue_size = ctx.shared.sched.len().await;
+                        let queue_size = ctx.sched.len().await;
                         let stop_ctx = stop::StopContext {
                             pages: ctx.state.stats.pages.load(Ordering::SeqCst),
                             items: ctx.state.stats.items.load(Ordering::SeqCst),
                             errors: ctx.state.stats.errors.load(Ordering::SeqCst),
                             in_flight: ctx.state.stats.in_flight.load(Ordering::SeqCst),
                             elapsed: ctx.state.stats.start.elapsed(),
                             queue_size,
                         };
                         if ctx.state.spider.until().should_stop(&stop_ctx) {
                             if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
@@ -437,46 +434,46 @@ impl Engine {
                         let limit = if let Some(ref pool) = autoscale {
                             pool.current_concurrency()
                         } else {
                             ctx.config.max_concurrent
                         };
                         if ctx.state.global_in_flight.load(Ordering::SeqCst) >= limit {
                             // ND-004-CORR/ND-007-PERF：已达并发上限，纯 Notify 驱动等待。
                             // 唤醒来源：process_response 末尾 notify_one（in-flight 下降）、
                             // autoscaler 扩容时 notify_one（limit 上升）。
                             // 不再使用 10ms timeout 轮询，避免 CPU 浪费。
-                            ctx.shared.work_notify.notified().await;
+                            ctx.work_notify.notified().await;
                             continue;
                         }
 
-                        let req = if let Some(req) = ctx.shared.sched.pop().await { req } else {
+                        let req = if let Some(req) = ctx.sched.pop().await { req } else {
                             if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                 return None;
                             }
                             // ND-004-CORR/ND-007-PERF：scheduler 空但仍有 in-flight，
                             // 纯 Notify 驱动等待新 work（follow 请求通过 process_response notify）。
-                            ctx.shared.work_notify.notified().await;
+                            ctx.work_notify.notified().await;
                             continue;
                         };
 
                         // 单 Spider：直接派发，无路由
                         ctx.state.global_in_flight.fetch_add(1, Ordering::SeqCst);
                         ctx.state.stats.in_flight.fetch_add(1, Ordering::SeqCst);
                         let ctx_c = ctx.clone();
                         let fut = async move {
                             let _g1 = engine::InFlightGuard {
                                 counter: ctx_c.state.global_in_flight.clone(),
-                                work_notify: ctx_c.shared.work_notify.clone(),
+                                work_notify: ctx_c.work_notify.clone(),
                             };
                             let _g2 = engine::InFlightGuard {
                                 counter: ctx_c.state.stats.in_flight.clone(),
-                                work_notify: ctx_c.shared.work_notify.clone(),
+                                work_notify: ctx_c.work_notify.clone(),
                             };
                             // 请求阶段 → 响应阶段（同级编排，process_request 不再内嵌 process_response）
                             if let Some(resp) = engine::process_request(&ctx_c, req).await {
                                 engine::process_response(&ctx_c, resp).await;
                             }
                         };
                         return Some((fut, (ctx, rx)));
                     }
                 }
             })
@@ -525,24 +522,23 @@ impl Engine {
 
         // 等待所有后台 checkpoint task 完成，避免 delete_checkpoint 后 task 又写入。
         while checkpoint_tasks.join_next().await.is_some() {}
 
         // abort autoscaler 后台 task
         if let Some(handle) = autoscaler_handle {
             handle.abort();
         }
 
         // pipeline 关闭：爬取结束后释放资源
-        if !ctx.shared.middleware_chain.is_empty() {
+        if !ctx.middleware_chain.is_empty() {
             let crawl_ctx = engine::build_crawl_context(&ctx);
-            ctx.shared
-                .middleware_chain
+            ctx.middleware_chain
                 .run_pipelines_close(&crawl_ctx)
                 .await;
         }
 
         spider.on_close().await;
 
         if let Some(ref store) = self.checkpoint_store {
             if let Err(e) = crate::storage::delete_checkpoint(&**store, &spider_name).await {
                 tracing::warn!("删除 checkpoint 失败: {}", e);
             }
