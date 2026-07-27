# Review package: 8c6311f..661739a

## Commits
661739a test: PR4 集成验证 Engine 配置聚合完整生命周期
05a921e refactor: re-export EngineConfig 到 wisp::crawl
70f2644 refactor: CrawlContext 增加 config 字段，中间件可访问完整配置
8859255 refactor: 删除 EngineShared，字段合并到 EngineContext 顶层
88e7769 refactor: EngineContext.config 改用 Arc<runner::EngineConfig>
a475d4a refactor: Engine 持有 Arc<EngineConfig> 并简化 EngineBuilder 为 4 字段
f8db29b fix: 为 fetcher 模式测试添加 feature gate
5970b08 refactor: 新增公开 EngineConfig 聚合 12 字段

## Files changed
 src/crawl/engine.rs             | 284 +++++++++++++++++-----------------
 src/crawl/middleware/builtin.rs |   1 +
 src/crawl/middleware/mod.rs     |  37 +++++
 src/crawl/mod.rs                |  10 +-
 src/crawl/runner.rs             | 328 ++++++++++++++++++++++++----------------
 src/fetcher/mod.rs              |   3 +
 6 files changed, 392 insertions(+), 271 deletions(-)

## Diff
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index 38de592..8571923 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -24,92 +24,76 @@ use super::stats::SpiderStats;
 use super::{
     auto, control, middleware, scheduler, CrawlEvent, CrawlState, CrawlStats, Request, Response,
     Spider,
 };
 use crate::error::Result;
 use crate::fetcher::FetchMode;
 use crate::http::Client;
 
 // === EngineContext: 打包所有共享状态 ===
 
-/// Engine 运行时上下文（单 Spider），由三层子结构组成。
+/// Engine 运行时上下文（单 Spider）。
 ///
-/// - `config`: 只读配置（从 Spider 提取，run 期间不变）
-/// - `shared`: 跨 task 共享的可变状态
-/// - `state`: per-run 可变状态
+/// PR4 重构：删除 EngineShared 子结构，字段直接展开到 EngineContext 顶层。
+/// 分为三组：
+/// - 只读配置：config (Arc<runner::EngineConfig>) + client (Arc<FetchClient>)
+/// - 跨 task 共享可变状态：sched / follow_tx / proxy_clients / control / work_notify / middleware_chain / rule_engine / cf_domain_locks
+/// - per-run 可变状态：state
 pub(crate) struct EngineContext {
-    pub config: EngineConfig,
-    pub shared: EngineShared,
-    pub state: EngineState,
-}
-
-/// 只读配置（从 Spider 提取，run 期间不变）。
-pub(crate) struct EngineConfig {
+    // === 只读配置 ===
+    pub config: Arc<crate::crawl::runner::EngineConfig>,
     pub client: Arc<crate::fetcher::FetchClient>,
-    pub fetch_mode: FetchMode,
-    pub max_concurrent: usize,
-    pub obey_robots: bool,
-    pub engine_max_pages: usize,
-    pub max_refetch_rounds: usize,
-    /// 网络错误重试上限（fetch 失败后同步重试，由 engine 维护 retry_count）。
-    ///
-    /// 与 `max_refetch_rounds`（响应中间件 Refetch 上限）独立：
-    /// - `max_retries`：fetch_page 失败后，engine 在 fetch_dispatch 内同步重试的次数上限
-    /// - `max_refetch_rounds`：响应成功后，process_response 内 Refetch 循环的次数上限
-    pub max_retries: u32,
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
@@ -132,24 +116,23 @@ async fn check_control_and_hook(ctx: &EngineContext, req: &Request) -> bool {
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
@@ -193,25 +176,24 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
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
@@ -281,39 +263,38 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
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
@@ -325,26 +306,26 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
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
@@ -369,27 +350,27 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
 #[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
 async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>) {
     let stats = &ctx.state.stats;
     let max_retries = ctx.config.max_retries;
     // 同步重试循环：clone 一次，循环内修改 retry_count
     let mut current_req = req.clone();
 
     loop {
         let proxy = current_req.proxy.clone();
         match fetch_page(
-            &ctx.config.client,
+            &ctx.client,
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
@@ -400,46 +381,45 @@ async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>
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
@@ -536,24 +516,25 @@ pub(crate) fn emit_error_event(ctx: &EngineContext, url: &str, err: &str) {
         });
     }
 }
 
 /// 从 EngineContext 构建中间件用的 CrawlContext 只读视图。
 pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
     middleware::CrawlContext {
         spider_name: ctx.state.spider.name().to_string(),
         fetch_mode: ctx.config.fetch_mode,
         max_concurrent: ctx.config.max_concurrent,
-        max_pages: ctx.config.engine_max_pages,
+        max_pages: ctx.config.max_pages,
         obey_robots: ctx.config.obey_robots,
         pages_crawled: ctx.state.stats.pages.load(Ordering::SeqCst),
         errors: ctx.state.stats.errors.load(Ordering::SeqCst),
+        config: Arc::clone(&ctx.config),
     }
 }
 
 /// 同步记录状态码计数（DashMap entry 原子累加，无 await）。
 #[doc(hidden)]
 pub fn record_status(stats: &Arc<SpiderStats>, status: u16) {
     stats
         .status_codes
         .entry(status)
         .and_modify(|c| {
@@ -950,42 +931,41 @@ mod tests {
             (vec![], vec![])
         }
     }
 
     /// 构造最小 EngineContext（单 Spider，Http 模式，无事件通道）。
     /// 返回上下文与对应 stats 的 Arc 克隆，便于测试断言计数器。
     fn make_ctx() -> (EngineContext, Arc<SpiderStats>) {
         let stats = Arc::new(SpiderStats::new());
         let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let ctx = EngineContext {
-            config: EngineConfig {
-                client: Arc::new(
-                    crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
-                        .expect("build fetch client"),
-                ),
+            config: Arc::new(crate::crawl::runner::EngineConfig {
                 fetch_mode: FetchMode::Http,
                 max_concurrent: 8,
                 obey_robots: false,
-                engine_max_pages: 100,
-                max_refetch_rounds: 5,
+                max_pages: 100,
                 max_retries: 3,
-            },
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
+                max_refetch_rounds: 5,
+                ..Default::default()
+            }),
+            client: Arc::new(
+                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
+                    .expect("build fetch client"),
+            ),
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
@@ -1069,42 +1049,41 @@ mod tests {
         let stats = Arc::new(SpiderStats::new());
         let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let mut chain = middleware::MiddlewareChain::new();
         chain
             .middlewares
             .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                 max_retries,
             )));
 
         let ctx = EngineContext {
-            config: EngineConfig {
-                client: Arc::new(
-                    crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
-                        .expect("build fetch client"),
-                ),
+            config: Arc::new(crate::crawl::runner::EngineConfig {
                 fetch_mode: FetchMode::Http,
                 max_concurrent: 8,
                 obey_robots: false,
-                engine_max_pages: 100,
-                max_refetch_rounds: 5,
+                max_pages: 100,
                 max_retries,
-            },
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
+                max_refetch_rounds: 5,
+                ..Default::default()
+            }),
+            client: Arc::new(
+                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
+                    .expect("build fetch client"),
+            ),
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
@@ -1178,47 +1157,45 @@ mod tests {
     fn make_ctx_auto(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
         let stats = Arc::new(SpiderStats::new());
         let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let mut chain = middleware::MiddlewareChain::new();
         chain
             .middlewares
             .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                 max_retries,
             )));
 
-        let fetch_config = crate::fetcher::FetchClientConfig {
-            max_concurrent_pages: 0, // 禁用浏览器池，Stealth 模式快速失败
-            ..Default::default()
-        };
-
         let ctx = EngineContext {
-            config: EngineConfig {
-                client: Arc::new(
-                    crate::fetcher::FetchClient::new(fetch_config).expect("build fetch client"),
-                ),
+            config: Arc::new(crate::crawl::runner::EngineConfig {
                 fetch_mode: FetchMode::Auto,
                 max_concurrent: 8,
                 obey_robots: false,
-                engine_max_pages: 100,
-                max_refetch_rounds: 5,
+                max_pages: 100,
                 max_retries,
+                max_refetch_rounds: 5,
+                ..Default::default()
+            }),
+            client: {
+                let fetch_config = crate::fetcher::FetchClientConfig {
+                    max_concurrent_pages: 0, // 禁用浏览器池，Stealth 模式快速失败
+                    ..Default::default()
+                };
+                Arc::new(crate::fetcher::FetchClient::new(fetch_config).expect("build fetch client"))
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
@@ -1239,55 +1216,55 @@ mod tests {
 
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
@@ -1353,123 +1330,122 @@ mod tests {
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
         Arc<SpiderStats>,
         tokio::sync::mpsc::Receiver<CrawlEvent>,
     ) {
         let stats = Arc::new(SpiderStats::new());
         let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
         let ctx = EngineContext {
-            config: EngineConfig {
-                client: Arc::new(
-                    crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
-                        .expect("build fetch client"),
-                ),
+            config: Arc::new(crate::crawl::runner::EngineConfig {
                 fetch_mode: FetchMode::Http,
                 max_concurrent: 8,
                 obey_robots: false,
-                engine_max_pages: 100,
-                max_refetch_rounds: 5,
+                max_pages: 100,
                 max_retries,
-            },
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
+                max_refetch_rounds: 5,
+                ..Default::default()
+            }),
+            client: Arc::new(
+                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
+                    .expect("build fetch client"),
+            ),
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
@@ -1638,11 +1614,37 @@ mod tests {
         let result = tx.try_send(3);
         assert!(matches!(
             result,
             Err(tokio::sync::mpsc::error::TrySendError::Full(3))
         ));
 
         // 消费一个后可再发送
         assert_eq!(rx.recv().await.unwrap(), 1);
         tx.try_send(3).unwrap();
     }
+
+    /// PR4 Task 4：验证 EngineContext.config 是 Arc<runner::EngineConfig>。
+    #[test]
+    fn test_engine_context_config_is_arc_runner_config() {
+        let (ctx, _stats) = make_ctx();
+        // 验证 config 字段类型为 Arc<crate::crawl::runner::EngineConfig>
+        let _config: &crate::crawl::runner::EngineConfig = &ctx.config;
+        // 验证 client 字段独立持有（不再嵌套在 config 中）
+        let _client: &Arc<crate::fetcher::FetchClient> = &ctx.client;
+        assert_eq!(ctx.config.fetch_mode, crate::fetcher::FetchMode::Http);
+        assert_eq!(ctx.config.max_concurrent, 8);
+    }
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
diff --git a/src/crawl/middleware/builtin.rs b/src/crawl/middleware/builtin.rs
index cb7ce63..b11c87b 100644
--- a/src/crawl/middleware/builtin.rs
+++ b/src/crawl/middleware/builtin.rs
@@ -807,20 +807,21 @@ mod tests {
 
     fn make_ctx() -> CrawlContext {
         CrawlContext {
             spider_name: "test".into(),
             fetch_mode: FetchMode::Http,
             max_concurrent: 8,
             max_pages: 1000,
             obey_robots: false,
             pages_crawled: 0,
             errors: 0,
+            config: std::sync::Arc::new(crate::crawl::runner::EngineConfig::default()),
         }
     }
 
     #[tokio::test]
     async fn test_ua_rotation_middleware() {
         let mw = UaRotationMiddleware::desktop();
         let ctx = make_ctx();
         let mut req = make_req();
         let action = mw.process_request(&mut req, &ctx).await;
         assert_eq!(action, MwAction::Modified);
diff --git a/src/crawl/middleware/mod.rs b/src/crawl/middleware/mod.rs
index bb8c99a..791b794 100644
--- a/src/crawl/middleware/mod.rs
+++ b/src/crawl/middleware/mod.rs
@@ -73,36 +73,47 @@ pub enum ErrorAction {
     Propagate,
     /// 重试此请求
     Retry,
     /// 忽略错误，跳过此请求
     Ignore,
 }
 
 /// 引擎上下文只读视图（暴露给中间件）。
 ///
 /// 中间件可读取引擎级配置和统计信息，用于决策（如根据已爬取页数调整策略）。
+/// PR4 重构：新增 config 字段（Arc<EngineConfig>），中间件可通过 config() 访问完整配置。
 #[derive(Debug, Clone)]
 pub struct CrawlContext {
     /// Spider 名称
     pub spider_name: String,
     /// 当前抓取模式
     pub fetch_mode: FetchMode,
     /// 最大并发数
     pub max_concurrent: usize,
     /// 最大爬取页数
     pub max_pages: usize,
     /// 是否遵守 robots.txt
     pub obey_robots: bool,
     /// 已爬取页数（只读快照）
     pub pages_crawled: usize,
     /// 错误数（只读快照）
     pub errors: usize,
+    /// 完整引擎配置（Arc 共享，中间件可访问任意配置字段）。
+    pub config: std::sync::Arc<crate::crawl::runner::EngineConfig>,
+}
+
+impl CrawlContext {
+    /// 获取完整引擎配置引用。
+    #[must_use]
+    pub fn config(&self) -> &crate::crawl::runner::EngineConfig {
+        &self.config
+    }
 }
 
 // === Middleware trait ===
 
 /// 请求/响应中间件（Downloader Middleware 等价）。
 ///
 /// 实现此 trait 以在请求发出前/响应返回后执行自定义逻辑。
 /// 所有方法都有默认实现，只需覆盖需要的方法。
 #[async_trait]
 pub trait Middleware: Send + Sync {
@@ -262,10 +273,36 @@ impl MiddlewareChain {
         }
     }
 
     /// 关闭所有 pipeline（爬取结束后调用）。
     pub(crate) async fn run_pipelines_close(&self, ctx: &CrawlContext) {
         for pipeline in &self.pipelines {
             pipeline.close(ctx).await;
         }
     }
 }
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    /// PR4 Task 6：验证 CrawlContext 持有 config 字段并提供 config() 方法。
+    #[test]
+    fn test_crawl_context_has_config_field() {
+        let config = std::sync::Arc::new(crate::crawl::runner::EngineConfig::default());
+        let ctx = CrawlContext {
+            spider_name: "test".to_string(),
+            fetch_mode: crate::fetcher::FetchMode::Http,
+            max_concurrent: 8,
+            max_pages: 100,
+            obey_robots: false,
+            pages_crawled: 0,
+            errors: 0,
+            config: std::sync::Arc::clone(&config),
+        };
+        // 验证 config() 方法返回 &EngineConfig
+        let cfg: &crate::crawl::runner::EngineConfig = ctx.config();
+        assert_eq!(cfg.max_concurrent, 8);
+        assert_eq!(cfg.max_pages, 1000); // default
+        assert_eq!(cfg.fetch_mode, crate::fetcher::FetchMode::Auto); // default
+    }
+}
diff --git a/src/crawl/mod.rs b/src/crawl/mod.rs
index 812e780..679ec63 100644
--- a/src/crawl/mod.rs
+++ b/src/crawl/mod.rs
@@ -20,21 +20,21 @@ pub use runtime::items;
 pub use runtime::output;
 pub use runtime::robots;
 pub use scheduling::scheduler;
 pub use scheduling::stop;
 
 pub use adaptive::AdaptiveTracker;
 pub use auto::ModeRuleEngine;
 pub use builder::{ClosureSpider, SpiderBuilder};
 pub use engine::{fetch_page, fetch_page_inner, record_status};
 pub use items::{Items, JsonlWriter};
-pub use runner::{Engine, EngineBuilder};
+pub use runner::{Engine, EngineBuilder, EngineConfig};
 pub use state::CrawlState;
 pub use stop::{
     FnStopCondition, MaxErrors, MaxItems, MaxPages, NeverStop, StopCondition, StopContext, Timeout,
 };
 
 use async_trait::async_trait;
 use serde_json::Value;
 use std::collections::{HashMap, HashSet};
 use std::sync::Arc;
 use std::time::Duration;
@@ -475,11 +475,19 @@ mod tests {
         assert_eq!(nodes.iter().count(), 1);
     }
 
     #[test]
     fn test_method_as_str_returns_standard_verbs() {
         assert_eq!(Method::Get.as_str(), "GET");
         assert_eq!(Method::Post.as_str(), "POST");
         assert_eq!(Method::Put.as_str(), "PUT");
         assert_eq!(Method::Delete.as_str(), "DELETE");
     }
+
+    /// PR4 Task 7：验证 wisp::crawl::EngineConfig 公开可访问。
+    #[test]
+    fn test_engine_config_public_accessible() {
+        let config = crate::crawl::EngineConfig::default();
+        assert_eq!(config.max_concurrent, 8);
+        assert_eq!(config.max_pages, 1000);
+    }
 }
diff --git a/src/crawl/runner.rs b/src/crawl/runner.rs
index 56dff5e..cb8f638 100644
--- a/src/crawl/runner.rs
+++ b/src/crawl/runner.rs
@@ -8,100 +8,133 @@ use std::time::Duration;
 use tokio::sync::Mutex;
 
 use super::stats::SpiderStats;
 use super::{
     auto, control, engine, middleware, robots, scheduler, stop, CrawlEvent, CrawlState, CrawlStats,
     CrawlStream, Request, Spider,
 };
 use crate::error::Result;
 use crate::fetcher::{FetchClient, FetchClientConfig};
 
+/// 所有只读引擎配置聚合（Arc 共享，构建后不可变）。
+///
+/// ARCH: 替代散落在 Engine/EngineConfig/EngineShared 的 26 字段。
+/// 构建后通过 Arc 共享，所有模块通过 `&Arc<EngineConfig>` 访问。
+#[derive(Debug, Clone)]
+pub struct EngineConfig {
+    // === 并发与配额 ===
+    /// 最大并发数。
+    pub max_concurrent: usize,
+    /// 最大爬取页数（引擎级兜底）。
+    pub max_pages: usize,
+    /// 最大错误数（达到此上限引擎停止）。
+    pub max_errors: usize,
+
+    // === 抓取行为 ===
+    /// 抓取模式（Http/Dynamic/Stealth/Auto）。
+    pub fetch_mode: crate::fetcher::FetchMode,
+    /// 是否遵守 robots.txt。
+    pub obey_robots: bool,
+    /// 网络错误重试上限（fetch 失败后同步重试）。
+    pub max_retries: u32,
+    /// 响应中间件 Refetch 最大轮数。
+    pub max_refetch_rounds: usize,
+    /// 下载延迟（每次请求前的等待时间）。
+    pub download_delay: Duration,
+
+    // === 检查点 ===
+    /// 检查点保存间隔（页数）。
+    pub checkpoint_interval: usize,
+    /// 检查点自定义名称（默认使用 spider name）。
+    pub checkpoint_name: Option<String>,
+
+    // === 自动模式规则 ===
+    /// Auto 模式 URL 正则规则（优先级最高，跳过嗅探）。
+    pub auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
+
+    // === HTTP 配置（含 proxy 子字段） ===
+    /// FetchClient 配置（HTTP 连接池/超时/浏览器等基础设施配置）。
+    pub fetch_client_config: crate::fetcher::FetchClientConfig,
+}
+
+impl Default for EngineConfig {
+    fn default() -> Self {
+        Self {
+            max_concurrent: 8,
+            max_pages: 1000,
+            max_errors: 1000,
+            fetch_mode: crate::fetcher::FetchMode::Auto,
+            obey_robots: true,
+            max_retries: 3,
+            max_refetch_rounds: 5,
+            download_delay: Duration::ZERO,
+            checkpoint_interval: 100,
+            checkpoint_name: None,
+            auto_rules: Vec::new(),
+            fetch_client_config: crate::fetcher::FetchClientConfig::default(),
+        }
+    }
+}
+
 /// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
 ///
-/// Task 3 重构：从"Spider 容器"变为"纯基础设施"。
+/// PR4 重构：Engine 持有 Arc<EngineConfig>，所有只读配置通过 config() 访问。
 /// - 不持有 Spider（删除 `spiders: Vec<Box<dyn Spider>>`）
 /// - 共享：HTTP client / Store（SQLite 缓存 + checkpoint）
 /// - 独立：每次 run 内部 Scheduler/去重/stats（per-Spider 隔离）
 /// - 控制：per-Engine `EngineControl`
-///
-/// ND-031-ARCH 修复：引擎配置（fetch_mode/obey_robots/max_retries/download_delay/auto_rules）
-/// 从 Spider trait 迁移到 Engine，职责分离：Spider 只关心解析逻辑，Engine 管理抓取行为。
 #[derive(Clone)]
 pub struct Engine {
+    /// 只读配置（Arc 共享给 Spider/Middleware）。
+    pub(crate) config: Arc<EngineConfig>,
     /// 共享 FetchClient（HTTP 连接池 + BrowserPool，跨 Spider 复用）
     pub(crate) fetch_client: Arc<FetchClient>,
     pub(crate) cache_store: Option<Arc<dyn crate::storage::Store>>,
-    pub(crate) max_concurrent: usize,
-    pub(crate) max_pages: usize,
-    pub(crate) max_refetch_rounds: usize,
     pub(crate) checkpoint_store: Option<Arc<dyn crate::storage::Store>>,
-    pub(crate) checkpoint_interval: usize,
     /// per-Engine 控制状态。
     pub(crate) control: Arc<control::EngineControl>,
     /// 自适应并发池（可选）。启用后 run_inner 动态调整并发数。
     pub(crate) autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
     /// 运行时并发保护：防止同一 Engine 实例并发调用 run/run_stream。
     /// 未来支持并发爬取时，移除此 guard 并将 EngineControl 改为 per-run 即可。
     pub(crate) running: Arc<AtomicBool>,
-    // === 引擎配置（ND-031-ARCH：从 Spider trait 迁移） ===
-    /// 抓取模式（Http/Dynamic/Stealth/Auto）。
-    pub(crate) fetch_mode: crate::fetcher::FetchMode,
-    /// 是否遵守 robots.txt。
-    pub(crate) obey_robots: bool,
-    /// 网络错误重试上限（fetch 失败后同步重试）。
-    pub(crate) max_retries: u32,
-    /// 下载延迟（每次请求前的等待时间）。
-    pub(crate) download_delay: Duration,
-    /// Auto 模式 URL 正则规则（优先级最高，跳过嗅探）。
-    pub(crate) auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
 }
 
 /// Engine 构造器（Builder 模式）。
+///
+/// PR4 重构：字段简化为 4 个（config + cache_store + checkpoint_store + autoscale）。
+/// 所有配置 setter 操作 self.config.xxx。
 pub struct EngineBuilder {
-    fetch_client_config: FetchClientConfig,
-    max_concurrent: usize,
-    max_pages: usize,
-    max_refetch_rounds: usize,
+    config: EngineConfig,
     cache_store: Option<Arc<dyn crate::storage::Store>>,
     checkpoint_store: Option<Arc<dyn crate::storage::Store>>,
-    checkpoint_interval: usize,
     autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
-    // === 引擎配置（ND-031-ARCH） ===
-    fetch_mode: crate::fetcher::FetchMode,
-    obey_robots: bool,
-    max_retries: u32,
-    download_delay: Duration,
-    auto_rules: Vec<(String, crate::fetcher::FetchMode)>,
 }
 
 impl Engine {
+    /// 获取只读配置引用（Arc 共享给中间件和 Spider）。
+    #[must_use]
+    pub fn config(&self) -> &Arc<EngineConfig> {
+        &self.config
+    }
+
     /// 创建 Engine builder（纯基础设施构造器）。
     ///
     /// 替代原 `Engine::new(spider)` / `Engine::spiders(vec)` / `Engine::builder(spider)`。
     /// Engine 不再持有 Spider，长期持有共享底层资源。
     #[must_use]
     pub fn infra() -> EngineBuilder {
         EngineBuilder {
-            fetch_client_config: FetchClientConfig::default(),
-            max_concurrent: 8,
-            max_pages: 1000,
-            max_refetch_rounds: 5,
+            config: EngineConfig::default(),
             cache_store: Some(Arc::new(crate::storage::MemoryStore::default())),
             checkpoint_store: Some(Arc::new(crate::storage::FileStore::default())),
-            checkpoint_interval: 100,
             autoscale: None,
-            // 引擎配置默认值（ND-031-ARCH：原 Spider trait 默认值）
-            fetch_mode: crate::fetcher::FetchMode::Auto,
-            obey_robots: true,
-            max_retries: 3,
-            download_delay: Duration::ZERO,
-            auto_rules: Vec::new(),
         }
     }
 
     /// 运行单个 Spider。返回 (统计, items)。
     ///
     /// 共享底层资源（HTTP/缓存/代理），Spider 内部独立 Scheduler/去重。
     /// 可多次调用：`engine.run(spider_a).await?; engine.run(spider_b).await?;`
     ///
     /// # 并发约束
     /// **不可并发调用**。`run` / `run_stream` 共享同一个 `EngineControl`，
@@ -203,28 +236,27 @@ impl Engine {
             }
         }
         let _guard = RunGuard(self.running.clone());
 
         // 重置 control（每次 run 清理上次状态）
         self.control.reset().await;
 
         let stats = Arc::new(SpiderStats::new());
         // ND-031-ARCH：引擎配置从 Engine 自身读取（而非 Spider trait 方法）
         let mut rule_engine = auto::ModeRuleEngine::new();
-        for (pattern, mode) in &self.auto_rules {
+        for (pattern, mode) in &self.config().auto_rules {
             rule_engine.add_user_rule(pattern, *mode)?;
         }
         let rule_engine = Arc::new(Mutex::new(rule_engine));
-        let fetch_mode = self.fetch_mode;
-        let max_concurrent = self.max_concurrent;
+        let fetch_mode = self.config().fetch_mode;
         let max_depth = spider.max_depth();
-        let obey_robots = self.obey_robots;
+        let obey_robots = self.config().obey_robots;
 
         // 复用 Engine 持有的共享 FetchClient（HTTP 连接池 + BrowserPool 跨 Spider 复用）
         let fetch_client = self.fetch_client.clone();
 
         let sched = Arc::new(scheduler::Scheduler::new());
         let robots_cache = Arc::new(robots::RobotsCache::new());
         let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
 
         // checkpoint 恢复（单 Spider）
         let spider_name = spider.name().to_string();
@@ -259,89 +291,79 @@ impl Engine {
             }
         }
 
         spider.on_start().await;
 
         // 默认中间件注入所需资源（在 ctx 字面量 move 这些 Arc 前提取）
         let mw_http_client = fetch_client.http_arc();
         let mw_robots_cache = robots_cache.clone();
 
         let ctx = Arc::new(engine::EngineContext {
-            config: engine::EngineConfig {
-                client: fetch_client,
-                fetch_mode,
-                max_concurrent,
-                obey_robots,
-                engine_max_pages: self.max_pages,
-                max_refetch_rounds: self.max_refetch_rounds,
-                max_retries: self.max_retries,
-            },
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
-                            delay: self.download_delay,
-                            obey_robots,
-                            allowed_domains: spider.allowed_domains(),
-                            max_depth,
-                            cache_store: self.cache_store.clone(),
-                            http_client: mw_http_client,
-                            robots_cache: mw_robots_cache,
-                            rule_engine: rule_engine.clone(),
-                            max_retries: self.max_retries,
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
+            config: Arc::clone(self.config()),
+            client: fetch_client,
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
@@ -353,46 +375,46 @@ impl Engine {
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
-                            >= ctx.config.engine_max_pages
+                            >= ctx.config.max_pages
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
@@ -412,67 +434,67 @@ impl Engine {
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
             .buffer_unordered(buffer_ceiling)
         };
 
         // 驱动流 + 定期 checkpoint
         tokio::pin!(stream);
         let mut pages_since_checkpoint = 0usize;
         // 跟踪后台 checkpoint task，run 结束时统一 await，避免 delete_checkpoint 后 task 仍写入。
         let mut checkpoint_tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
         while stream.next().await.is_some() {
             pages_since_checkpoint += 1;
-            if pages_since_checkpoint >= self.checkpoint_interval {
+            if pages_since_checkpoint >= self.config().checkpoint_interval {
                 if let Some(ref store) = self.checkpoint_store {
                     // OPTIMIZE: spawn 后台执行，主循环不等待；失败 tracing::warn + 补发 CrawlEvent::Error。
                     // 旧实现直接 await persist_spider_checkpoint，慢存储会阻塞主循环拖慢吞吐。
                     let store = Arc::clone(store);
                     let spider_name = spider_name.clone();
                     let sched = Arc::clone(&sched);
                     let stats = Arc::clone(&ctx.state.stats);
                     let tx = ctx.state.tx.clone();
                     checkpoint_tasks.spawn(async move {
                         if let Err(e) = engine::persist_spider_checkpoint(
@@ -500,24 +522,23 @@ impl Engine {
 
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
@@ -529,57 +550,69 @@ impl Engine {
             status_codes,
             ctx.state.start,
         ))
     }
 }
 
 impl EngineBuilder {
     /// 设置最大并发数。
     #[must_use]
     pub fn max_concurrent(mut self, n: usize) -> Self {
-        self.max_concurrent = n;
+        self.config.max_concurrent = n;
         self
     }
     /// 设置最大爬取页数。
     #[must_use]
     pub fn max_pages(mut self, n: usize) -> Self {
-        self.max_pages = n;
+        self.config.max_pages = n;
+        self
+    }
+    /// 设置最大错误数（达到此上限引擎停止）。
+    #[must_use]
+    pub fn max_errors(mut self, n: usize) -> Self {
+        self.config.max_errors = n;
         self
     }
     /// 设置 FetchClient 配置（HTTP 连接池/超时/浏览器等基础设施配置，跨 Spider 共享）。
     #[must_use]
     pub fn fetch_client_config(mut self, config: FetchClientConfig) -> Self {
-        self.fetch_client_config = config;
+        self.config.fetch_client_config = config;
         self
     }
     /// 设置代理（作用于共享 FetchClient 的所有 HTTP 请求）。
     #[must_use]
     pub fn proxy(mut self, proxy: &str) -> Self {
-        self.fetch_client_config.proxy = Some(proxy.to_string());
+        self.config.fetch_client_config.proxy = Some(proxy.to_string());
         self
     }
     /// 设置中间件 Refetch 最大轮数（默认 5）。
     #[must_use]
     pub fn max_refetch_rounds(mut self, n: usize) -> Self {
-        self.max_refetch_rounds = n;
+        self.config.max_refetch_rounds = n;
         self
     }
     /// 设置响应缓存存储（注入 CacheMiddleware，永不过期）。
     /// 想要 TTL 的用户应通过 `Spider::middlewares()` 自定义 `CacheMiddleware`。
     pub fn cache_store(mut self, store: Arc<dyn crate::storage::Store>) -> Self {
         self.cache_store = Some(store);
         self
     }
     /// 设置检查点存储（定期保存爬取进度）。
     pub fn checkpoint(mut self, s: Arc<dyn crate::storage::Store>, interval: usize) -> Self {
         self.checkpoint_store = Some(s);
-        self.checkpoint_interval = interval;
+        self.config.checkpoint_interval = interval;
+        self
+    }
+    /// 设置检查点自定义名称（默认使用 spider name）。
+    #[must_use]
+    pub fn checkpoint_name(mut self, name: impl Into<String>) -> Self {
+        self.config.checkpoint_name = Some(name.into());
         self
     }
 
     /// 启用自适应并发池。min 为初始/下限，max 为上限。
     /// 启用后 run_inner 会启动后台 autoscaler，根据饱和度动态调整并发数。
     #[must_use]
     pub fn autoscale(mut self, min: usize, max: usize) -> Self {
         self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(
             min,
             max,
@@ -595,90 +628,81 @@ impl EngineBuilder {
         min: usize,
         max: usize,
         config: crate::crawl::runtime::autoscale::AutoscaleConfig,
     ) -> Self {
         self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(
             min, max, config,
         ));
         self
     }
 
-    // === 引擎配置方法（ND-031-ARCH：从 Spider trait 迁移） ===
+    // === 引擎配置方法 ===
 
     /// 设置抓取模式（Http/Dynamic/Stealth/Auto，默认 Auto）。
     ///
     /// 这是引擎行为配置，决定如何抓取页面，与 Spider 的解析逻辑无关。
     #[must_use]
     pub fn fetch_mode(mut self, mode: crate::fetcher::FetchMode) -> Self {
-        self.fetch_mode = mode;
+        self.config.fetch_mode = mode;
         self
     }
 
     /// 是否遵守 robots.txt（默认 true）。
     #[must_use]
     pub fn obey_robots(mut self, obey: bool) -> Self {
-        self.obey_robots = obey;
+        self.config.obey_robots = obey;
         self
     }
 
     /// 设置网络错误重试上限（默认 3）。
     ///
     /// fetch_page 失败后，engine 在 fetch_dispatch 内同步重试，计数 `req.retry_count`。
     #[must_use]
     pub fn max_retries(mut self, n: u32) -> Self {
-        self.max_retries = n;
+        self.config.max_retries = n;
         self
     }
 
     /// 设置下载延迟（默认 0，即无延迟）。
     #[must_use]
     pub fn download_delay(mut self, d: Duration) -> Self {
-        self.download_delay = d;
+        self.config.download_delay = d;
         self
     }
 
     /// 设置下载延迟（毫秒）。
     #[must_use]
     pub fn download_delay_ms(mut self, ms: u64) -> Self {
-        self.download_delay = Duration::from_millis(ms);
+        self.config.download_delay = Duration::from_millis(ms);
         self
     }
 
     /// Auto 模式：添加 URL 正则规则（优先级最高，跳过嗅探）。
     ///
     /// 匹配该规则的 URL 直接使用指定模式，不经过 Auto 嗅探。
     #[must_use]
     pub fn auto_rule(mut self, pattern: &str, mode: crate::fetcher::FetchMode) -> Self {
-        self.auto_rules.push((pattern.to_string(), mode));
+        self.config.auto_rules.push((pattern.to_string(), mode));
         self
     }
 
     /// 构建引擎实例。
     pub fn build(self) -> Result<Engine> {
-        let fetch_client = Arc::new(FetchClient::new(self.fetch_client_config)?);
+        let fetch_client = Arc::new(FetchClient::new(self.config.fetch_client_config.clone())?);
         Ok(Engine {
+            config: Arc::new(self.config),
             fetch_client,
             cache_store: self.cache_store,
-            max_concurrent: self.max_concurrent,
-            max_pages: self.max_pages,
-            max_refetch_rounds: self.max_refetch_rounds,
             checkpoint_store: self.checkpoint_store,
-            checkpoint_interval: self.checkpoint_interval,
             control: Arc::new(control::EngineControl::new()),
             autoscale: self.autoscale,
             running: Arc::new(AtomicBool::new(false)),
-            // 引擎配置（ND-031-ARCH）
-            fetch_mode: self.fetch_mode,
-            obey_robots: self.obey_robots,
-            max_retries: self.max_retries,
-            download_delay: self.download_delay,
-            auto_rules: self.auto_rules,
         })
     }
 }
 
 #[cfg(test)]
 mod tests {
     /// Task 3：验证 UnboundedReceiver 可直接 try_recv drain，无需 Mutex 包装。
     ///
     /// Receiver 是单消费者类型，本身串行化访问；原实现 `Arc<Mutex<UnboundedReceiver>>`
     /// 的 Mutex 是冗余的，本测试确认 try_recv drain 模式可行。
@@ -721,11 +745,57 @@ mod tests {
 
         assert_eq!(
             flag.load(Ordering::SeqCst),
             0,
             "主循环不应等待 spawn 的任务"
         );
 
         tokio::time::sleep(Duration::from_millis(80)).await;
         assert_eq!(flag.load(Ordering::SeqCst), 1, "spawn 任务应完成");
     }
+
+    /// PR4 Task 8：集成验证 — Engine 完整生命周期使用 config() 路径访问配置。
+    #[tokio::test]
+    async fn test_engine_full_lifecycle_with_config_accessor() {
+        use futures::StreamExt;
+        use std::time::Duration;
+
+        struct OkSpider;
+        #[async_trait::async_trait]
+        impl super::super::Spider for OkSpider {
+            fn name(&self) -> &'static str { "ok" }
+            fn start_urls(&self) -> Vec<String> { vec![] }
+            async fn handle(&self, _resp: super::super::Response) -> (Vec<serde_json::Value>, Vec<super::super::Request>) {
+                (vec![], vec![])
+            }
+        }
+
+        let engine = super::Engine::infra()
+            .max_concurrent(2)
+            .max_pages(1)
+            .max_errors(10)
+            .fetch_mode(crate::fetcher::FetchMode::Http)
+            .obey_robots(false)
+            .download_delay_ms(10)
+            .build()
+            .unwrap();
+
+        // 验证所有 config 字段通过 config() 访问
+        assert_eq!(engine.config().max_concurrent, 2);
+        assert_eq!(engine.config().max_pages, 1);
+        assert_eq!(engine.config().max_errors, 10);
+        assert_eq!(engine.config().fetch_mode, crate::fetcher::FetchMode::Http);
+        assert!(!engine.config().obey_robots);
+        assert_eq!(engine.config().download_delay, Duration::from_millis(10));
+
+        // 验证 run_stream 不 panic
+        let mut stream = engine.run_stream(OkSpider).events();
+        let mut got_done = false;
+        while let Some(event) = stream.next().await {
+            if matches!(event, super::super::CrawlEvent::Done(_)) {
+                got_done = true;
+                break;
+            }
+        }
+        assert!(got_done, "应收到 Done 事件");
+    }
 }
diff --git a/src/fetcher/mod.rs b/src/fetcher/mod.rs
index b361994..f9db6aa 100644
--- a/src/fetcher/mod.rs
+++ b/src/fetcher/mod.rs
@@ -425,51 +425,54 @@ mod tests {
 
         assert_eq!(fetcher.mode(), FetchMode::Http);
         assert_eq!(
             fetcher.config().proxy.as_deref(),
             Some("http://127.0.0.1:7897")
         );
         assert_eq!(fetcher.config().timeout, Duration::from_mins(1));
         assert_eq!(fetcher.config().emulation, Some(Profile::Firefox128));
     }
 
+    #[cfg(feature = "stealth")]
     #[test]
     fn test_fetcher_builder_stealth() {
         let fetcher = Fetcher::stealth()
             .headless(true)
             .human_mode(true)
             .challenge_timeout(Duration::from_mins(1))
             .proxy("http://127.0.0.1:7897")
             .build()
             .expect("build fetcher");
 
         assert_eq!(fetcher.mode(), FetchMode::Stealth);
         assert!(fetcher.config().headless);
         assert!(fetcher.config().human_mode);
         assert_eq!(fetcher.config().challenge_timeout, Duration::from_mins(1));
     }
 
+    #[cfg(feature = "browser")]
     #[test]
     fn test_fetcher_builder_dynamic() {
         let fetcher = Fetcher::dynamic()
             .headless(false)
             .wait_for(".content")
             .extra_wait_ms(2000)
             .build()
             .expect("build fetcher");
 
         assert_eq!(fetcher.mode(), FetchMode::Dynamic);
         assert!(!fetcher.config().headless);
         assert_eq!(fetcher.config().wait_for.as_deref(), Some(".content"));
         assert_eq!(fetcher.config().extra_wait_ms, 2000);
     }
 
+    #[cfg(feature = "browser")]
     #[test]
     fn test_fetcher_builder_block_ads() {
         let fetcher = Fetcher::dynamic()
             .block_ads()
             .block_domains(&["analytics.example.com"])
             .build()
             .expect("build fetcher");
 
         let blocker = fetcher.config().domain_blocker.as_ref().unwrap();
         assert!(blocker.is_ad_blocking_enabled());
