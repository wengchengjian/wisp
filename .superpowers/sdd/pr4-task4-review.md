# Review package: a475d4a..88e7769

## Commits
88e7769 refactor: EngineContext.config 改用 Arc<runner::EngineConfig>

## Files changed
 src/crawl/engine.rs | 113 ++++++++++++++++++++++++++--------------------------
 src/crawl/runner.rs |  14 ++-----
 2 files changed, 60 insertions(+), 67 deletions(-)

## Diff
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index 38de592..bb67f9f 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -24,47 +24,33 @@ use super::stats::SpiderStats;
 use super::{
     auto, control, middleware, scheduler, CrawlEvent, CrawlState, CrawlStats, Request, Response,
     Spider,
 };
 use crate::error::Result;
 use crate::fetcher::FetchMode;
 use crate::http::Client;
 
 // === EngineContext: 打包所有共享状态 ===
 
-/// Engine 运行时上下文（单 Spider），由三层子结构组成。
+/// Engine 运行时上下文（单 Spider），由三层组成。
 ///
-/// - `config`: 只读配置（从 Spider 提取，run 期间不变）
+/// - `config`: 只读配置（Arc 共享，构建后不可变）
+/// - `client`: 共享 FetchClient（跨 task 复用）
 /// - `shared`: 跨 task 共享的可变状态
 /// - `state`: per-run 可变状态
 pub(crate) struct EngineContext {
-    pub config: EngineConfig,
+    pub config: Arc<crate::crawl::runner::EngineConfig>,
+    pub client: Arc<crate::fetcher::FetchClient>,
     pub shared: EngineShared,
     pub state: EngineState,
 }
 
-/// 只读配置（从 Spider 提取，run 期间不变）。
-pub(crate) struct EngineConfig {
-    pub client: Arc<crate::fetcher::FetchClient>,
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
-
 /// 跨 task 共享的可变状态。
 pub(crate) struct EngineShared {
     pub sched: Arc<scheduler::Scheduler>,
     pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
     // OPTIMIZE: follow_rx 移出 EngineShared，由 runner.rs unfold 状态持有。
     // 旧实现 `Arc<Mutex<UnboundedReceiver>>` 串行化所有 follow drain，但 Receiver 是
     // 单消费者类型，无需 Mutex；保留 Mutex 只增加锁争用与无谓 await 开销。
     /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）。
     ///
     /// ND-009-SEC：使用 moka::sync::Cache 限制最大条目数，防止攻击者注入大量
@@ -369,21 +355,21 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
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
             &ctx.shared.rule_engine,
             &ctx.shared.proxy_clients,
             &ctx.shared.cf_domain_locks,
         )
         .await
         {
             Ok(resp) => {
@@ -536,21 +522,21 @@ pub(crate) fn emit_error_event(ctx: &EngineContext, url: &str, err: &str) {
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
     }
 }
 
 /// 同步记录状态码计数（DashMap entry 原子累加，无 await）。
 #[doc(hidden)]
 pub fn record_status(stats: &Arc<SpiderStats>, status: u16) {
     stats
@@ -950,32 +936,33 @@ mod tests {
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
+                max_refetch_rounds: 5,
+                ..Default::default()
+            }),
+            client: Arc::new(
+                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
+                    .expect("build fetch client"),
+            ),
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
                 rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
                 cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
             },
@@ -1069,32 +1056,33 @@ mod tests {
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
+                max_refetch_rounds: 5,
+                ..Default::default()
+            }),
+            client: Arc::new(
+                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
+                    .expect("build fetch client"),
+            ),
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new(chain),
                 rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
                 cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
             },
@@ -1178,36 +1166,36 @@ mod tests {
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
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new(chain),
                 rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
                 cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
@@ -1426,32 +1414,33 @@ mod tests {
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
+                max_refetch_rounds: 5,
+                ..Default::default()
+            }),
+            client: Arc::new(
+                crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
+                    .expect("build fetch client"),
+            ),
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new({
                     let mut chain = middleware::MiddlewareChain::new();
                     chain
                         .middlewares
@@ -1638,11 +1627,23 @@ mod tests {
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
 }
diff --git a/src/crawl/runner.rs b/src/crawl/runner.rs
index 022a509..ccce301 100644
--- a/src/crawl/runner.rs
+++ b/src/crawl/runner.rs
@@ -241,21 +241,20 @@ impl Engine {
         self.control.reset().await;
 
         let stats = Arc::new(SpiderStats::new());
         // ND-031-ARCH：引擎配置从 Engine 自身读取（而非 Spider trait 方法）
         let mut rule_engine = auto::ModeRuleEngine::new();
         for (pattern, mode) in &self.config().auto_rules {
             rule_engine.add_user_rule(pattern, *mode)?;
         }
         let rule_engine = Arc::new(Mutex::new(rule_engine));
         let fetch_mode = self.config().fetch_mode;
-        let max_concurrent = self.config().max_concurrent;
         let max_depth = spider.max_depth();
         let obey_robots = self.config().obey_robots;
 
         // 复用 Engine 持有的共享 FetchClient（HTTP 连接池 + BrowserPool 跨 Spider 复用）
         let fetch_client = self.fetch_client.clone();
 
         let sched = Arc::new(scheduler::Scheduler::new());
         let robots_cache = Arc::new(robots::RobotsCache::new());
         let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
 
@@ -292,29 +291,22 @@ impl Engine {
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
-                engine_max_pages: self.config().max_pages,
-                max_refetch_rounds: self.config().max_refetch_rounds,
-                max_retries: self.config().max_retries,
-            },
+            config: Arc::clone(self.config()),
+            client: fetch_client,
             shared: engine::EngineShared {
                 sched: sched.clone(),
                 follow_tx,
                 // ND-009-SEC：moka::Cache 限制 proxy client 缓存最大 1024 条
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: self.control.clone(),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: {
                     // 默认中间件链：按 fetch_mode + spider 配置注入（详见 builtin::default_middlewares）
                     let defaults = middleware::builtin::default_middlewares(
@@ -401,21 +393,21 @@ impl Engine {
 
                         // OPTIMIZE: 直接 try_recv drain follow channel，无 Mutex 锁争用。
                         // Receiver 是单消费者类型，串行化访问无意义。
                         while let Ok(req) = rx.try_recv() {
                             ctx.shared.sched.push(req).await;
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
                         let queue_size = ctx.shared.sched.len().await;
