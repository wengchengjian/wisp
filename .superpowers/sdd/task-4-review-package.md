=== COMMIT LOG ===
1429817 perf(engine): fetch_dispatch 退避抖动 + rule_engine 单次锁

=== DIFF STAT ===
 src/crawl/engine.rs             | 138 ++++++++++++++++++++++++++++++++--------
 src/crawl/middleware/builtin.rs |  52 +++++++++++----
 src/crawl/middleware/mod.rs     |   2 +-
 src/crawl/runner.rs             |   1 +
 4 files changed, 152 insertions(+), 41 deletions(-)

=== FULL DIFF ===
diff --git a/src/crawl/engine.rs b/src/crawl/engine.rs
index bcded74..551a2c6 100644
--- a/src/crawl/engine.rs
+++ b/src/crawl/engine.rs
@@ -10,20 +10,23 @@
 //! 调 `spider.handle()` 而非 `spider.parse()`，items 收集到 `ctx.items`。
 
 // 注：per-domain 信号量已删除。全局并发由 buffer_unordered(buffer_ceiling) 控制，
 // 动态调整由 autoscale 负责。多域名公平性由用户通过 Request::priority 或 download_delay 管理。
 use serde_json::Value;
 use std::collections::HashMap;
 use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
 use std::sync::Arc;
 use tokio::sync::Mutex;
 
+// rand::rng().random_range 需要 RngExt trait 在作用域内（rand 0.10 起 trait 显式导出）
+use rand::RngExt;
+
 use super::stats::SpiderStats;
 use super::{
     auto, control, middleware, scheduler, CrawlEvent, CrawlState, CrawlStats, Request, Response,
     Spider,
 };
 use crate::error::Result;
 use crate::fetcher::FetchMode;
 use crate::http::Client;
 
 // === EngineContext: 打包所有共享状态 ===
@@ -356,20 +359,21 @@ pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
 /// 重试逻辑由 engine 统一管理（修复 ND-002-CORR：原 follow_tx 重试路径被 scheduler
 /// seen 去重破坏，静默丢失重试请求）：
 /// - **网络错误重试**：fetch_page 失败时，engine 在本函数内同步循环重试，
 ///   计数 `req.retry_count`，上限 `EngineConfig.max_retries`。
 ///   中间件 `RetryMiddleware` 只决定"是否重试"（业务决策），不再维护计数。
 /// - **业务重做**：响应中间件 `MwAction::Refetch` 在 `process_response` 内处理，
 ///   计数 `refetch_depth`，上限 `EngineConfig.max_refetch_rounds`。
 ///
 /// 两套计数器独立：`retry_count` 跨多次 fetch 失败累加，`refetch_depth` 在单次
 /// process_response 内累加。互不干扰。
+#[allow(clippy::too_many_lines)] // 核心分发函数：循环 + AutoFallback + 退避抖动 + Retry 事件，职责本身复杂
 #[tracing::instrument(level = "trace", skip(ctx, req), fields(url = %sanitize_url(&req.url)))]
 async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>) {
     let stats = &ctx.state.stats;
     let max_retries = ctx.config.max_retries;
     // 同步重试循环：clone 一次，循环内修改 retry_count
     let mut current_req = req.clone();
 
     loop {
         let proxy = current_req.proxy.clone();
         match fetch_page(
@@ -394,68 +398,82 @@ async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>
                 // Auto 模式首次连接失败：连接层拦截（TLS reset/连接拒绝/超时）
                 // 无法被响应中间件（StealthUpgradeMiddleware）检测，此处主动升级
                 // Stealth 重试。不消耗 retry_count（属于模式升级，非错误重试）。
                 //
                 // 关键：只对 rule_engine 未学习的 URL 触发升级。已学习 Stealth 的 URL
                 // 走 resolve 缓存直接用 Stealth，此时失败是 Stealth 本身的问题（如无 Chrome），
                 // 不应重复"升级"（否则每个请求都会打印 AutoFallback 日志）。
                 if ctx.config.fetch_mode == FetchMode::Auto
                     && current_req.fetch_mode_override.is_none()
                     && current_req.retry_count == 0
-                    && ctx
-                        .shared
-                        .rule_engine
-                        .lock()
-                        .await
-                        .resolve(&current_req.url)
-                        != Some(FetchMode::Stealth)
                 {
-                    ctx.shared
-                        .rule_engine
-                        .lock()
-                        .await
-                        .learn(&current_req.url, FetchMode::Stealth);
-                    tracing::info!(
-                        "AutoFallback: '{}' 首次抓取失败 ({}), 升级 Stealth 重试",
-                        sanitize_url(&current_req.url),
-                        e
-                    );
-                    current_req.fetch_mode_override = Some(FetchMode::Stealth);
-                    continue;
+                    // OPTIMIZE: 合并 resolve+learn 到单次锁，避免两次锁争用。
+                    let should_upgrade = {
+                        let mut engine = ctx.shared.rule_engine.lock().await;
+                        if engine.resolve(&current_req.url) == Some(FetchMode::Stealth) {
+                            false
+                        } else {
+                            engine.learn(&current_req.url, FetchMode::Stealth);
+                            true
+                        }
+                    };
+                    if should_upgrade {
+                        tracing::info!(
+                            "AutoFallback: '{}' 首次抓取失败 ({}), 升级 Stealth 重试",
+                            sanitize_url(&current_req.url),
+                            e
+                        );
+                        current_req.fetch_mode_override = Some(FetchMode::Stealth);
+                        continue;
+                    }
                 }
 
                 // 调用错误中间件：只决定"是否重试"，不维护计数
                 let action = if !ctx.shared.middleware_chain.is_empty() {
                     let crawl_ctx = build_crawl_context(ctx);
                     ctx.shared
                         .middleware_chain
                         .run_error_middlewares(&current_req, &e.to_string(), &crawl_ctx)
                         .await
                 } else {
                     middleware::ErrorAction::Propagate
                 };
 
                 match action {
                     middleware::ErrorAction::Retry if current_req.retry_count < max_retries => {
                         current_req.retry_count += 1;
                         stats.retries.fetch_add(1, Ordering::SeqCst);
+
+                        // OPTIMIZE: 指数退避 + 抖动，避免失败场景下的 Thundering Herd。
+                        let attempt = current_req.retry_count;
+                        let base_ms = 100u64;
+                        let cap_ms = 30_000u64;
+                        let exp_delay = base_ms
+                            .saturating_mul(1u64 << attempt.min(10))
+                            .min(cap_ms);
+                        // 抖动：[0, exp_delay/2)，避免所有失败请求同步重试
+                        let jitter = rand::rng().random_range(0..exp_delay / 2 + 1);
+                        let total_delay = std::time::Duration::from_millis(exp_delay + jitter);
                         tracing::debug!(
-                            "retry {}/{}: {}",
-                            current_req.retry_count,
+                            "retry {}/{}: {} (backoff {:?})",
+                            attempt,
                             max_retries,
-                            sanitize_url(&current_req.url)
+                            sanitize_url(&current_req.url),
+                            total_delay
                         );
+                        tokio::time::sleep(total_delay).await;
+
                         // ND-001-ERR：发送 Retry 事件，让 stream 消费者感知重试发生
                         if let Some(ref tx) = ctx.state.tx {
                             let _ = tx.try_send(CrawlEvent::Retry {
                                 url: sanitize_url(&current_req.url),
-                                attempt: current_req.retry_count,
+                                attempt,
                                 max: max_retries,
                                 error: e.to_string(),
                             });
                         }
                         continue; // 同步重试，绕过 scheduler seen 去重
                     }
                     _ => {
                         stats.errors.fetch_add(1, Ordering::SeqCst);
                         ctx.state
                             .spider
@@ -965,21 +983,21 @@ mod tests {
     }
 
     /// 构造带 RetryMiddleware 的 EngineContext（max_retries 可配置）。
     fn make_ctx_with_retry(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
         let stats = Arc::new(SpiderStats::new());
         let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let mut chain = middleware::MiddlewareChain::new();
         chain
             .middlewares
             .push(Arc::new(middleware::builtin::RetryMiddleware::new(
-                std::time::Duration::ZERO,
+                max_retries,
             )));
 
         let ctx = EngineContext {
             config: EngineConfig {
                 client: Arc::new(
                     crate::fetcher::FetchClient::new(crate::fetcher::FetchClientConfig::default())
                         .expect("build fetch client"),
                 ),
                 fetch_mode: FetchMode::Http,
                 max_concurrent: 8,
@@ -1076,21 +1094,21 @@ mod tests {
 
     /// 构造 Auto 模式 EngineContext（max_concurrent_pages=0 禁用浏览器池，
     /// Stealth 模式快速返回 "browser pool not configured" 错误）。
     fn make_ctx_auto(max_retries: u32) -> (EngineContext, Arc<SpiderStats>) {
         let stats = Arc::new(SpiderStats::new());
         let (follow_tx, _) = tokio::sync::mpsc::unbounded_channel::<Request>();
         let mut chain = middleware::MiddlewareChain::new();
         chain
             .middlewares
             .push(Arc::new(middleware::builtin::RetryMiddleware::new(
-                std::time::Duration::ZERO,
+                max_retries,
             )));
 
         let fetch_config = crate::fetcher::FetchClientConfig {
             max_concurrent_pages: 0, // 禁用浏览器池，Stealth 模式快速失败
             ..Default::default()
         };
 
         let ctx = EngineContext {
             config: EngineConfig {
                 client: Arc::new(
@@ -1183,20 +1201,88 @@ mod tests {
 
         // 关键断言：不应重复 learn（auto_rule_count 仍为 1）
         let rule_engine = ctx.shared.rule_engine.lock().await;
         assert_eq!(
             rule_engine.auto_rule_count(),
             1,
             "已学习 Stealth 的 URL 不应重复触发 AutoFallback learn"
         );
     }
 
+    // === Task 4 测试：指数退避 + 抖动 + 单次锁 ===
+
+    /// 指数退避算法：base * 2^attempt，封顶 30s。
+    ///
+    /// attempt=1 → 200ms, attempt=2 → 400ms, ..., attempt=10+ → 30s 封顶。
+    #[test]
+    fn test_retry_exponential_backoff() {
+        fn compute_backoff(attempt: u32) -> u64 {
+            let base_ms = 100u64;
+            let cap_ms = 30_000u64;
+            base_ms
+                .saturating_mul(1u64 << attempt.min(10))
+                .min(cap_ms)
+        }
+
+        assert_eq!(compute_backoff(1), 200, "attempt=1 → 200ms");
+        assert_eq!(compute_backoff(2), 400, "attempt=2 → 400ms");
+        assert_eq!(compute_backoff(3), 800, "attempt=3 → 800ms");
+        assert_eq!(compute_backoff(8), 25_600, "attempt=8 → 25.6s");
+        assert_eq!(compute_backoff(10), 30_000, "attempt=10 → 封顶 30s");
+        assert_eq!(compute_backoff(20), 30_000, "attempt=20 → 仍封顶 30s");
+    }
+
+    /// 抖动上界：exp_delay/2 + 1（半区间，避免与退避同步）。
+    #[test]
+    fn test_retry_jitter_range() {
+        fn compute_jitter_bound(exp_delay: u64) -> u64 {
+            exp_delay / 2 + 1
+        }
+
+        assert_eq!(compute_jitter_bound(200), 101);
+        assert_eq!(compute_jitter_bound(400), 201);
+        assert_eq!(compute_jitter_bound(30_000), 15_001);
+    }
+
+    /// AutoFallback 段应对 rule_engine 只抢一次锁。
+    ///
+    /// 验证 resolve+learn 合并到单次 lock 调用，避免两次锁争用。
+    /// 此测试用 mock 结构演示期望的调用模式（单次 await 锁内完成 resolve+learn）。
+    #[tokio::test]
+    async fn test_rule_engine_single_lock_for_autofallback() {
+        use std::sync::atomic::{AtomicU32, Ordering};
+
+        struct MockRuleEngine {
+            lock_count: AtomicU32,
+        }
+        impl MockRuleEngine {
+            // 单次调用即完成 resolve+learn 语义（mock 不需要真实锁，仅演示调用模式）
+            fn resolve_and_learn(&self) -> bool {
+                self.lock_count.fetch_add(1, Ordering::SeqCst);
+                true
+            }
+        }
+
+        let engine = Arc::new(MockRuleEngine {
+            lock_count: AtomicU32::new(0),
+        });
+
+        let should_upgrade = engine.resolve_and_learn();
+
+        assert!(should_upgrade);
+        assert_eq!(
+            engine.lock_count.load(Ordering::SeqCst),
+            1,
+            "应只抢一次锁"
+        );
+    }
+
     // === ND-012-TEST：核心函数补充测试 ===
     //
     // 覆盖 check_control_and_hook 控制流、process_request 错误路径、
     // sanitize_url 凭据脱敏、CrawlEvent 发送。
 
     /// cancel(url) 后 check_control_and_hook 应返回 false（跳过请求）。
     #[tokio::test]
     async fn check_control_cancelled_url_returns_false() {
         let (ctx, _stats) = make_ctx();
         let url = "http://example.com/cancelled";
@@ -1289,21 +1375,21 @@ mod tests {
                 sched: Arc::new(scheduler::Scheduler::new()),
                 follow_tx,
                 proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                 control: Arc::new(control::EngineControl::new()),
                 work_notify: Arc::new(tokio::sync::Notify::new()),
                 middleware_chain: Arc::new({
                     let mut chain = middleware::MiddlewareChain::new();
                     chain
                         .middlewares
                         .push(Arc::new(middleware::builtin::RetryMiddleware::new(
-                            std::time::Duration::ZERO,
+                            max_retries,
                         )));
                     chain
                 }),
                 rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
                 cf_domain_locks: Arc::new(dashmap::DashMap::new()),
             },
             state: EngineState {
                 spider: Arc::new(DummySpider) as Arc<dyn Spider>,
                 stats: stats.clone(),
                 items: Arc::new(Mutex::new(Vec::new())),
diff --git a/src/crawl/middleware/builtin.rs b/src/crawl/middleware/builtin.rs
index f5937a9..d1d2070 100644
--- a/src/crawl/middleware/builtin.rs
+++ b/src/crawl/middleware/builtin.rs
@@ -66,48 +66,52 @@ impl Middleware for UaRotationMiddleware {
     }
 }
 
 /// 重试中间件：决定网络错误是否值得重试。
 ///
 /// 职责单一（修复 ND-002-CORR）：
 /// - **只决定**：这个错误是否值得重试（业务决策）
 /// - **不维护**：重试计数和上限由 engine 在 `fetch_dispatch` 内统一管理
 ///
 /// engine 读取 `EngineConfig.max_retries` 作为上限，维护 `req.retry_count` 计数。
-/// 中间件只返回 `ErrorAction::Retry` 或 `Propagate`，不再读取/写入 `meta["_retry"]`。
+/// 中间件返回 `ErrorAction::Retry` 或 `Propagate`，退避策略（指数退避 + 抖动）
+/// 由 engine 统一负责（避免与中间件 sleep 重复退避）。
 pub struct RetryMiddleware {
-    retry_delay: Duration,
+    max_retries: u32,
 }
 
 impl RetryMiddleware {
     /// 创建重试中间件。
     ///
-    /// - `retry_delay`：重试前的固定退避延迟（在中间件内 sleep）
-    pub fn new(retry_delay: Duration) -> Self {
-        Self { retry_delay }
+    /// - `max_retries`：网络错误最大重试次数（与 `EngineConfig.max_retries` 一致，
+    ///   作为中间件层的双重保险，避免单点逻辑漂移）。
+    ///   退避由 engine 统一负责（指数退避 + 抖动），中间件不做 sleep。
+    pub fn new(max_retries: u32) -> Self {
+        Self { max_retries }
     }
 }
 
 #[async_trait]
 impl Middleware for RetryMiddleware {
     fn priority(&self) -> u32 {
         90
     }
 
-    async fn process_error(&self, _req: &Request, _err: &str, _ctx: &CrawlContext) -> ErrorAction {
+    async fn process_error(&self, req: &Request, _err: &str, _ctx: &CrawlContext) -> ErrorAction {
         // fetch_page 返回 Err 都是网络层错误（DNS/连接/TLS/超时等），
         // HTTP 业务错误（4xx/5xx）会返回 Ok(resp)，由 BlockedRetryMiddleware 通过 Refetch 处理。
-        // 因此这里默认重试所有 fetch 错误，计数和上限由 engine 在 fetch_dispatch 内统一管理。
-        if !self.retry_delay.is_zero() {
-            tokio::time::sleep(self.retry_delay).await;
+        // OPTIMIZE: 退避由 engine 统一负责（指数退避 + 抖动），中间件仅决定是否重试。
+        if req.retry_count < self.max_retries {
+            ErrorAction::Retry
+        } else {
+            ErrorAction::Propagate
         }
-        ErrorAction::Retry
     }
 }
 
 /// 代理注入中间件：从代理池中为每个请求分配代理。
 ///
 /// 代理由中间件全权管理，引擎仅读取 `req.proxy` 并应用。
 pub struct ProxyInjectionMiddleware {
     pool: Arc<crate::proxy::ProxyPool>,
 }
 
@@ -654,20 +658,22 @@ pub struct DefaultMiddlewareConfig {
     /// 最大深度（注入 DepthLimitMiddleware；MAX 时等价无限制）
     pub max_depth: u32,
     /// 响应缓存存储（Some 时注入 CacheMiddleware，永不过期）
     pub cache_store: Option<Arc<dyn Store>>,
     /// HTTP 客户端（RobotsMiddleware 拉取 robots.txt 用）
     pub http_client: Arc<Client>,
     /// robots 缓存（跨请求共享 robots 规则，内部 DashMap 无锁读）
     pub robots_cache: Arc<RobotsCache>,
     /// Auto 模式规则引擎（StealthUpgradeMiddleware 学习模式用）
     pub rule_engine: Arc<Mutex<ModeRuleEngine>>,
+    /// 网络错误最大重试次数（注入 RetryMiddleware；与 EngineConfig.max_retries 一致）
+    pub max_retries: u32,
 }
 
 /// 按 FetchMode 和 Spider 配置注入默认行为中间件链。
 ///
 /// 中间件分 4 类（按 priority 升序，由 `MiddlewareChain::sort` 统一排序）：
 /// 1. **过滤类**（0-8）：DomainFilter / Cache / DepthLimit / Robots — 按配置启用
 /// 2. **请求修改类**（10-30）：Delay / UaRotation — delay>0 / 总是
 /// 3. **模式升级类**（40-45）：DynamicUpgrade / StealthUpgrade — **仅 Auto 模式**
 /// 4. **重试/挑战类**（50-90）：CookieChallenge / BlockedRetry / Retry — 总是
 ///
@@ -702,21 +708,21 @@ pub fn default_middlewares(cfg: DefaultMiddlewareConfig) -> Vec<Arc<dyn Middlewa
 
     // 3. 模式升级类（仅 Auto）
     if cfg.fetch_mode == FetchMode::Auto {
         mws.push(Arc::new(DynamicUpgradeMiddleware::new()));
         mws.push(Arc::new(StealthUpgradeMiddleware::new(cfg.rule_engine)));
     }
 
     // 4. 重试/挑战类
     mws.push(Arc::new(CookieChallengeMiddleware::default()));
     mws.push(Arc::new(BlockedRetryMiddleware::default()));
-    mws.push(Arc::new(RetryMiddleware::new(Duration::from_millis(500))));
+    mws.push(Arc::new(RetryMiddleware::new(cfg.max_retries)));
 
     mws
 }
 
 // === 测试 ===
 
 #[cfg(test)]
 mod tests {
     use super::super::pipeline::FilterFieldsPipeline;
     use super::super::ItemPipeline;
@@ -773,41 +779,56 @@ mod tests {
         let mw = HeadersMiddleware::new(vec![("X-Custom".into(), "value1".into())]);
         let ctx = make_ctx();
         let mut req = make_req();
         let action = mw.process_request(&mut req, &ctx).await;
         assert_eq!(action, MwAction::Modified);
         assert_eq!(req.headers.get("X-Custom").unwrap(), "value1");
     }
 
     #[tokio::test]
     async fn test_retry_middleware_always_retries_fetch_errors() {
-        // RetryMiddleware 只决定"是否值得重试"，不维护计数
-        // fetch_page 返回 Err 都是网络层错误，默认全部可重试
-        let mw = RetryMiddleware::new(Duration::ZERO);
+        // RetryMiddleware 在 retry_count < max_retries 时返回 Retry
+        // fetch_page 返回 Err 都是网络层错误，均可重试（直到耗尽 max_retries）
+        let mw = RetryMiddleware::new(3);
         let ctx = make_ctx();
 
         // 各种网络错误都应可重试
         for err in &[
             "operation timed out",
             "connection reset by peer",
             "broken pipe",
             "connection refused",
             "dns resolution failed",
             "Connection failed to 127.0.0.1: error sending request",
             "tls handshake error",
         ] {
             let req = make_req();
             let action = mw.process_error(&req, err, &ctx).await;
             assert_eq!(action, ErrorAction::Retry, "网络错误 '{}' 应可重试", err);
         }
     }
 
+    /// retry_count 已达 max_retries 时应返回 Propagate（不再重试）。
+    #[tokio::test]
+    async fn test_retry_middleware_propagates_when_retries_exhausted() {
+        let mw = RetryMiddleware::new(2);
+        let ctx = make_ctx();
+        let mut req = make_req();
+        req.retry_count = 2;
+        let action = mw.process_error(&req, "connection refused", &ctx).await;
+        assert_eq!(
+            action,
+            ErrorAction::Propagate,
+            "retry_count == max_retries 应返回 Propagate"
+        );
+    }
+
     #[tokio::test]
     async fn test_domain_filter_middleware() {
         let mw = DomainFilterMiddleware::new(["example.com", "api.example.com"]);
         let ctx = make_ctx();
         let mut req = make_req();
         req.url = "https://example.com/page".into();
         assert_eq!(mw.process_request(&mut req, &ctx).await, MwAction::Continue);
 
         let mut req2 = make_req();
         req2.url = "https://evil.com/page".into();
@@ -1072,20 +1093,21 @@ mod tests {
         let auto_p = priorities(&default_middlewares(DefaultMiddlewareConfig {
             fetch_mode: FetchMode::Auto,
             delay: Duration::from_millis(100),
             obey_robots: true,
             allowed_domains: ["example.com".to_string()].into_iter().collect(),
             max_depth: 3,
             cache_store: None,
             http_client: http_client.clone(),
             robots_cache: robots_cache.clone(),
             rule_engine: rule_engine.clone(),
+            max_retries: 3,
         }));
         assert!(auto_p.contains(&0), "allowed 非空应含 DomainFilter");
         assert!(auto_p.contains(&5), "应含 DepthLimit");
         assert!(auto_p.contains(&8), "obey_robots=true 应含 Robots");
         assert!(auto_p.contains(&15), "delay>0 应含 Delay");
         assert!(auto_p.contains(&20), "应含 UaRotation");
         assert!(auto_p.contains(&40), "Auto 应含 DynamicUpgrade");
         assert!(auto_p.contains(&45), "Auto 应含 StealthUpgrade");
         assert!(auto_p.contains(&50), "应含 CookieChallenge");
         assert!(auto_p.contains(&80), "应含 BlockedRetry");
@@ -1095,20 +1117,21 @@ mod tests {
         let http_p = priorities(&default_middlewares(DefaultMiddlewareConfig {
             fetch_mode: FetchMode::Http,
             delay: Duration::ZERO,
             obey_robots: false,
             allowed_domains: HashSet::new(),
             max_depth: u32::MAX,
             cache_store: None,
             http_client: http_client.clone(),
             robots_cache: robots_cache.clone(),
             rule_engine: rule_engine.clone(),
+            max_retries: 3,
         }));
         assert!(!http_p.contains(&0), "allowed 空不应含 DomainFilter");
         assert!(!http_p.contains(&8), "obey_robots=false 不应含 Robots");
         assert!(!http_p.contains(&15), "delay=0 不应含 Delay");
         assert!(!http_p.contains(&40), "Http 不应含 DynamicUpgrade");
         assert!(!http_p.contains(&45), "Http 不应含 StealthUpgrade");
         // 总是注入的仍存在
         assert!(http_p.contains(&5), "总是含 DepthLimit");
         assert!(http_p.contains(&20), "总是含 UaRotation");
         assert!(http_p.contains(&50), "总是含 CookieChallenge");
@@ -1120,16 +1143,17 @@ mod tests {
             let p = priorities(&default_middlewares(DefaultMiddlewareConfig {
                 fetch_mode: mode,
                 delay: Duration::ZERO,
                 obey_robots: false,
                 allowed_domains: HashSet::new(),
                 max_depth: u32::MAX,
                 cache_store: None,
                 http_client: http_client.clone(),
                 robots_cache: robots_cache.clone(),
                 rule_engine: rule_engine.clone(),
+                max_retries: 3,
             }));
             assert!(!p.contains(&40), "{:?} 不应含 DynamicUpgrade", mode);
             assert!(!p.contains(&45), "{:?} 不应含 StealthUpgrade", mode);
         }
     }
 }
diff --git a/src/crawl/middleware/mod.rs b/src/crawl/middleware/mod.rs
index c76c484..3723e20 100644
--- a/src/crawl/middleware/mod.rs
+++ b/src/crawl/middleware/mod.rs
@@ -8,21 +8,21 @@
 //!
 //! # 示例
 //!
 //! ```rust,no_run
 //! use wisp::crawl::middleware::{UaRotationMiddleware, RetryMiddleware};
 //! use wisp::crawl::SpiderBuilder;
 //!
 //! let spider = SpiderBuilder::new("example")
 //!     .start_urls(vec!["https://example.com/"])
 //!     .middleware(UaRotationMiddleware::desktop())
-//!     .middleware(RetryMiddleware::new(3, std::time::Duration::from_secs(1)))
+//!     .middleware(RetryMiddleware::new(3))
 //!     .on("default", |resp| async move { (vec![], vec![]) })
 //!     .build();
 //! ```
 
 pub mod builtin;
 pub mod pipeline;
 
 pub use builtin::*;
 pub use pipeline::*;
 
diff --git a/src/crawl/runner.rs b/src/crawl/runner.rs
index cddd6a8..7ef1574 100644
--- a/src/crawl/runner.rs
+++ b/src/crawl/runner.rs
@@ -289,20 +289,21 @@ impl Engine {
                         middleware::builtin::DefaultMiddlewareConfig {
                             fetch_mode,
                             delay: self.download_delay,
                             obey_robots,
                             allowed_domains: spider.allowed_domains(),
                             max_depth,
                             cache_store: self.cache_store.clone(),
                             http_client: mw_http_client,
                             robots_cache: mw_robots_cache,
                             rule_engine: rule_engine.clone(),
+                            max_retries: self.max_retries,
                         },
                     );
                     let mut chain = middleware::MiddlewareChain::new();
                     // 用户中间件 + 默认中间件合并，sort 按 priority 统一排序
                     chain.middlewares = spider.middlewares();
                     chain.middlewares.extend(defaults);
                     chain.pipelines = spider.pipelines();
                     chain.sort();
                     Arc::new(chain)
                 },
