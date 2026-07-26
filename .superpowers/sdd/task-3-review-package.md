=== COMMIT LOG ===
b1d5c88 perf(runner): follow_rx 移入 unfold 状态移除 Mutex

=== DIFF STAT ===
 src/crawl/engine.rs | 16 +++++++---------
 src/crawl/runner.rs | 38 ++++++++++++++++++++++++++++++--------
 2 files changed, 37 insertions(+), 17 deletions(-)

=== FULL DIFF ===
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index bb7fb13..bcded74 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -52,21 +52,23 @@ pub(crate) struct EngineConfig {
     /// 与 `max_refetch_rounds`（响应中间件 Refetch 上限）独立：
     /// - `max_retries`：fetch_page 失败后，engine 在 fetch_dispatch 内同步重试的次数上限
     /// - `max_refetch_rounds`：响应成功后，process_response 内 Refetch 循环的次数上限
     pub max_retries: u32,
 }
 
 /// 跨 task 共享的可变状态。
 pub(crate) struct EngineShared {
     pub sched: Arc<scheduler::Scheduler>,
     pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
-    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<Request>>>,
+    // OPTIMIZE: follow_rx 移出 EngineShared，由 runner.rs unfold 状态持有。
+    // 旧实现 `Arc<Mutex<UnboundedReceiver>>` 串行化所有 follow drain，但 Receiver 是
+    // 单消费者类型，无需 Mutex；保留 Mutex 只增加锁争用与无谓 await 开销。
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
@@ -847,38 +849,37 @@ mod tests {
         }
         async fn handle(&self, _resp: Response) -> (Vec<Value>, Vec<Request>) {
             (vec![], vec![])
         }
     }
 
     /// 构造最小 EngineContext（单 Spider，Http 模式，无事件通道）。
     /// 返回上下文与对应 stats 的 Arc 克隆，便于测试断言计数器。
     fn make_ctx() -> (EngineContext, Arc<SpiderStats>) {
         let stats = Arc::new(SpiderStats::new());
-        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
+        let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let ctx = EngineContext {
             config: EngineConfig {
                 client: Arc::new(
                     crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                         .expect("build fetch client"),
                 ),
                 fetch_mode: FetchMode::Http,
                 max_concurrent: 8,
                 obey_robots: false,
                 engine_max_pages: 100,
                 max_refetch_rounds: 5,
                 max_retries: 3,
             },
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
-                follow_rx: Arc::new(Mutex::new(follow_rx)),
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
                 rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
                 cf_domain_locks: Arc::new(dashmap::DashMap::new()),
             },
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
@@ -959,21 +960,21 @@ mod tests {
         assert!(
             state.seen_urls.contains("https://example.com/b"),
             "seen_urls 必须包含已爬 URL b，当前 seen = {:?}",
             state.seen_urls
         );
     }
 
     /// 构造带 RetryMiddleware 的 EngineContext（max_retries 可配置）。
     fn make_ctx_with_retry(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
         let stats = Arc::new(SpiderStats::new());
-        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
+        let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let mut chain = middleware::MiddlewareChain::new();
         chain
             .middlewares
             .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                 std::time::Duration::ZERO,
             )));
 
         let ctx = EngineContext {
             config: EngineConfig {
                 client: Arc::new(
@@ -983,21 +984,20 @@ mod tests {
                 fetch_mode: FetchMode::Http,
                 max_concurrent: 8,
                 obey_robots: false,
                 engine_max_pages: 100,
                 max_refetch_rounds: 5,
                 max_retries,
             },
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
-                follow_rx: Arc::new(Mutex::new(follow_rx)),
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new(chain),
                 rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
                 cf_domain_locks: Arc::new(dashmap::DashMap::new()),
             },
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
@@ -1071,21 +1071,21 @@ mod tests {
             0,
             "max_retries=0 时不应重试"
         );
         assert_eq!(stats.errors.load(Ordering::SeqCst), 1, "应计入一次错误");
     }
 
     /// 构造 Auto 模式 EngineContext（max_concurrent_pages=0 禁用浏览器池，
     /// Stealth 模式快速返回 "browser pool not configured" 错误）。
     fn make_ctx_auto(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
         let stats = Arc::new(SpiderStats::new());
-        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
+        let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let mut chain = middleware::MiddlewareChain::new();
         chain
             .middlewares
             .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                 std::time::Duration::ZERO,
             )));
 
         let fetch_config = crate::fetcher::FetchClientConfig {
             max_concurrent_pages: 0, // 禁用浏览器池，Stealth 模式快速失败
             ..Default::default()
@@ -1099,21 +1099,20 @@ mod tests {
                 fetch_mode: FetchMode::Auto,
                 max_concurrent: 8,
                 obey_robots: false,
                 engine_max_pages: 100,
                 max_refetch_rounds: 5,
                 max_retries,
             },
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
-                follow_rx: Arc::new(Mutex::new(follow_rx)),
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new(chain),
                 rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
                 cf_domain_locks: Arc::new(dashmap::DashMap::new()),
             },
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
@@ -1264,39 +1263,38 @@ mod tests {
 
     /// 构造带事件通道的 EngineContext，返回 (ctx, stats, rx) 用于消费事件。
     fn make_ctx_with_tx(
         max_retries: u32,
     ) -> (
         EngineContext,
         Arc<SpiderStats>,
         tokio::sync::mpsc::Receiver<CrawlEvent>,
     ) {
         let stats = Arc::new(SpiderStats::new());
-        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
+        let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let (tx, rx) = tokio::sync::mpsc::channel::<CrawlEvent>(128);
         let ctx = EngineContext {
             config: EngineConfig {
                 client: Arc::new(
                     crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                         .expect("build fetch client"),
                 ),
                 fetch_mode: FetchMode::Http,
                 max_concurrent: 8,
                 obey_robots: false,
                 engine_max_pages: 100,
                 max_refetch_rounds: 5,
                 max_retries,
             },
             shared: EngineShared {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
-                follow_rx: Arc::new(Mutex::new(follow_rx)),
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new({
                     let mut chain = middleware::MiddlewareChain::new();
                     chain
                         .middlewares
                         .push(Arc::new(middleware::builtin::RetryMiddleware::new(
                             std::time::Duration::ZERO,
                         )));
diff --git a/src/crawl/runner.rs b/src/crawl/runner.rs
index 2b2fdf8..cddd6a8 100644
--- a/src/crawl/runner.rs
+++ b/src/crawl/runner.rs
@@ -272,21 +272,20 @@ impl Engine {
                 fetch_mode,
                 max_concurrent,
                 obey_robots,
                 engine_max_pages: self.max_pages,
                 max_refetch_rounds: self.max_refetch_rounds,
                 max_retries: self.max_retries,
             },
             shared: engine::EngineShared {
                 sched: sched.clone(),
                 follow_tx,
-                follow_rx: Arc::new(Mutex::new(follow_rx)),
                 // ND-009-SEC：moka::Cache 限制 proxy client 缓存最大 1024 条
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: self.control.clone(),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: {
                     // 默认中间件链：按 fetch_mode + spider 配置注入（详见 builtin::default_middlewares）
                     let defaults = middleware::builtin::default_middlewares(
                         middleware::builtin::DefaultMiddlewareConfig {
                             fetch_mode,
                             delay: self.download_delay,
@@ -348,37 +347,37 @@ impl Engine {
         // 构建并发流：单 Spider，无路由
         let stream = {
             let ctx = ctx.clone();
             let autoscale = self.autoscale.clone();
             // buffer_unordered 的 ceiling：autoscale 启用时用 max_concurrency()，否则用 max_concurrent
             let buffer_ceiling = if let Some(ref pool) = autoscale {
                 pool.max_concurrency()
             } else {
                 ctx.config.max_concurrent
             };
-            stream::unfold((), move |_| {
-                let ctx = ctx.clone();
+            // OPTIMIZE: follow_rx move 进 unfold 状态。UnboundedReceiver 是单消费者，
+            // 无需 Mutex 串行化；旧实现 `Arc<Mutex<UnboundedReceiver>>` 的锁是冗余的。
+            stream::unfold((ctx, follow_rx), move |(ctx, mut rx)| {
                 let autoscale = autoscale.clone();
                 async move {
                     loop {
                         if ctx.shared.control.is_shutdown()
                             || ctx.state.abort_flag.load(Ordering::SeqCst)
                         {
                             return None;
                         }
 
-                        // drain follow channel
-                        let mut rx_guard = ctx.shared.follow_rx.lock().await;
-                        while let Ok(req) = rx_guard.try_recv() {
+                        // OPTIMIZE: 直接 try_recv drain follow channel，无 Mutex 锁争用。
+                        // Receiver 是单消费者类型，串行化访问无意义。
+                        while let Ok(req) = rx.try_recv() {
                             ctx.shared.sched.push(req).await;
                         }
-                        drop(rx_guard);
 
                         // 引擎级 max_pages 兜底
                         let pages = ctx.state.stats.pages.load(Ordering::SeqCst);
                         if pages + ctx.state.global_in_flight.load(Ordering::SeqCst)
                             >= ctx.config.engine_max_pages
                         {
                             if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 {
                                 return None;
                             }
                             tokio::task::yield_now().await;
@@ -448,21 +447,21 @@ impl Engine {
                             };
                             let _g2 = engine::InFlightGuard {
                                 counter: ctx_c.state.stats.in_flight.clone(),
                                 work_notify: ctx_c.shared.work_notify.clone(),
                             };
                             // 请求阶段 → 响应阶段（同级编排，process_request 不再内嵌 process_response）
                             if let Some(resp) = engine::process_request(&ctx_c, req).await {
                                 engine::process_response(&ctx_c, resp).await;
                             }
                         };
-                        return Some((fut, ()));
+                        return Some((fut, (ctx, rx)));
                     }
                 }
             })
             .buffer_unordered(buffer_ceiling)
         };
 
         // 驱动流 + 定期 checkpoint
         tokio::pin!(stream);
         let mut pages_since_checkpoint = 0usize;
         while stream.next().await.is_some() {
@@ -645,10 +644,33 @@ impl EngineBuilder {
             running: Arc::new(AtomicBool::new(false)),
             // 引擎配置（ND-031-ARCH）
             fetch_mode: self.fetch_mode,
             obey_robots: self.obey_robots,
             max_retries: self.max_retries,
             download_delay: self.download_delay,
             auto_rules: self.auto_rules,
         })
     }
 }
+
+#[cfg(test)]
+mod tests {
+    /// Task 3：验证 UnboundedReceiver 可直接 try_recv drain，无需 Mutex 包装。
+    ///
+    /// Receiver 是单消费者类型，本身串行化访问；原实现 `Arc<Mutex<UnboundedReceiver>>`
+    /// 的 Mutex 是冗余的，本测试确认 try_recv drain 模式可行。
+    #[tokio::test]
+    async fn test_follow_rx_drained_without_mutex() {
+        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<i32>();
+
+        tx.send(1).unwrap();
+        tx.send(2).unwrap();
+        tx.send(3).unwrap();
+
+        let mut drained = Vec::new();
+        while let Ok(v) = rx.try_recv() {
+            drained.push(v);
+        }
+
+        assert_eq!(drained, vec![1, 2, 3]);
+    }
+}
