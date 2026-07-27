## Task 4: 删除 engine.rs 内部 EngineConfig，EngineContext 改用 Arc<runner::EngineConfig>

**Files:**
- Modify: `src/crawl/engine.rs:32-92`（删除 pub(crate) EngineConfig，EngineContext 字段重构）
- Modify: `src/crawl/engine.rs:370-493`（fetch_dispatch 中 ctx.config.client → ctx.client）
- Modify: `src/crawl/engine.rs:541-551`（build_crawl_context 中 ctx.config.xxx 访问）
- Modify: `src/crawl/engine.rs:893-1438`（4 个测试辅助函数更新）
- Modify: `src/crawl/runner.rs:268-321`（run_inner 创建 EngineContext 改用 Arc<runner::EngineConfig>）

**Interfaces:**
- Consumes: Task 1-3 的 `runner::EngineConfig`、Task 2 的 `Engine::config()`
- Produces: `EngineContext.config: Arc<runner::EngineConfig>`，`EngineContext.client: Arc<FetchClient>`

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/engine.rs` 的 tests 模块中追加测试（验证 EngineContext 字段类型正确）：

```rust
    /// PR4 Task 4：验证 EngineContext.config 是 Arc<runner::EngineConfig>。
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
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_context_config_is_arc_runner_config`
Expected: FAIL（EngineContext.config 仍是 engine::EngineConfig，client 字段未独立）

- [ ] **Step 3: 写最小实现**

**3a. 替换 engine.rs 第 32-92 行（EngineContext + EngineConfig + EngineShared + EngineState）：**

```rust
// === EngineContext: 打包所有共享状态 ===

/// Engine 运行时上下文（单 Spider），由三层组成。
///
/// - `config`: 只读配置（Arc 共享，构建后不可变）
/// - `client`: 共享 FetchClient（跨 task 复用）
/// - `shared`: 跨 task 共享的可变状态
/// - `state`: per-run 可变状态
pub(crate) struct EngineContext {
    pub config: Arc<crate::crawl::runner::EngineConfig>,
    pub client: Arc<crate::fetcher::FetchClient>,
    pub shared: EngineShared,
    pub state: EngineState,
}

/// 跨 task 共享的可变状态。
pub(crate) struct EngineShared {
    pub sched: Arc<scheduler::Scheduler>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<Request>,
    /// 代理 Client 缓存（key=proxy URL，避免每请求重建 Client）。
    pub proxy_clients: Arc<moka::sync::Cache<String, Arc<Client>>>,
    pub control: Arc<control::EngineControl>,
    pub work_notify: Arc<tokio::sync::Notify>,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    /// 域名级 CF 挑战锁：防止初始并发请求全部走浏览器。
    pub cf_domain_locks: Arc<moka::sync::Cache<String, Arc<tokio::sync::Mutex<()>>>>,
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
```

注意：删除了原 `pub(crate) struct EngineConfig`（7 字段）。EngineShared 和 EngineState 暂时保留（Task 5 删除 EngineShared）。

**3b. 更新 fetch_dispatch 中 ctx.config.client 访问（engine.rs:379）：**

```rust
        match fetch_page(
            &ctx.client,  // 原 ctx.config.client
            &current_req,
            proxy.as_deref(),
            ctx.config.fetch_mode,
            &ctx.shared.rule_engine,
            &ctx.shared.proxy_clients,
            &ctx.shared.cf_domain_locks,
        )
        .await
```

**3c. 更新 build_crawl_context（engine.rs:541-551）：**

```rust
/// 从 EngineContext 构建中间件用的 CrawlContext 只读视图。
pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
    middleware::CrawlContext {
        spider_name: ctx.state.spider.name().to_string(),
        fetch_mode: ctx.config.fetch_mode,
        max_concurrent: ctx.config.max_concurrent,
        max_pages: ctx.config.max_pages,  // 原 engine_max_pages
        obey_robots: ctx.config.obey_robots,
        pages_crawled: ctx.state.stats.pages.load(Ordering::SeqCst),
        errors: ctx.state.stats.errors.load(Ordering::SeqCst),
    }
}
```

**3d. 更新 process_response 中 ctx.config.max_refetch_rounds 访问（engine.rs:224, 232, 236）：**

这些访问通过 Arc deref 仍可用，不需改动（字段名相同）。

**3e. 更新 runner.rs run_inner 创建 EngineContext（runner.rs:268-321）：**

```rust
        let ctx = Arc::new(engine::EngineContext {
            config: Arc::clone(self.config()),
            client: fetch_client.clone(),
            shared: engine::EngineShared {
                sched: sched.clone(),
                follow_tx,
                proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
                control: self.control.clone(),
                work_notify: Arc::new(tokio::sync::Notify::new()),
                middleware_chain: {
                    let defaults = middleware::builtin::default_middlewares(
                        middleware::builtin::DefaultMiddlewareConfig {
                            fetch_mode,
                            delay: self.config().download_delay,
                            obey_robots,
                            allowed_domains: spider.allowed_domains(),
                            max_depth,
                            cache_store: self.cache_store.clone(),
                            http_client: mw_http_client,
                            robots_cache: mw_robots_cache,
                            rule_engine: rule_engine.clone(),
                            max_retries: self.config().max_retries,
                        },
                    );
                    let mut chain = middleware::MiddlewareChain::new();
                    chain.middlewares = spider.middlewares();
                    chain.middlewares.extend(defaults);
                    chain.pipelines = spider.pipelines();
                    chain.sort();
                    Arc::new(chain)
                },
                rule_engine,
                cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            },
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
```

注意：`engine_max_pages: self.config().max_pages` 已在 build_crawl_context 中处理（Task 3 改为 ctx.config.max_pages）。

**3f. 更新 4 个测试辅助函数（engine.rs:917-953, 1029-1073, 1139-1187, 1386-1438）：**

每个 make_ctx / make_ctx_with_retry / make_ctx_auto / make_ctx_with_tx 中，原 `config: EngineConfig { client, fetch_mode, ... }` 改为：

```rust
            config: Arc::new(crate::crawl::runner::EngineConfig {
                fetch_mode: FetchMode::Http,
                max_concurrent: 8,
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
            shared: EngineShared { ... },  // 保持不变
            state: EngineState { ... },  // 保持不变
```

对 `make_ctx_auto`，fetch_mode 改为 `FetchMode::Auto`，并设置 fetch_client_config：

```rust
            config: Arc::new(crate::crawl::runner::EngineConfig {
                fetch_mode: FetchMode::Auto,
                max_concurrent: 8,
                obey_robots: false,
                max_pages: 100,
                max_retries,
                max_refetch_rounds: 5,
                fetch_client_config: crate::fetcher::FetchClientConfig {
                    max_concurrent_pages: 0,
                    ..Default::default()
                },
                ..Default::default()
            }),
            client: Arc::new(
                crate::fetcher::FetchClient::new(
                    crate::crawl::runner::EngineConfig::default().fetch_client_config,
                ).expect("build fetch client"),
            ),
```

注意：make_ctx_auto 的 client 应使用 max_concurrent_pages=0 的 config 构建。简化为：

```rust
            client: {
                let fetch_config = crate::fetcher::FetchClientConfig {
                    max_concurrent_pages: 0,
                    ..Default::default()
                };
                Arc::new(crate::fetcher::FetchClient::new(fetch_config).expect("build fetch client"))
            },
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_context_config_is_arc_runner_config`
Expected: PASS

Run: `cargo test --lib`
Expected: 现有测试全绿（4 个 make_ctx 测试函数更新后，所有依赖测试通过）

- [ ] **Step 5: 提交**

```bash
git add src/crawl/engine.rs src/crawl/runner.rs
git commit -m "refactor: EngineContext.config 改用 Arc<runner::EngineConfig>"
```

---

