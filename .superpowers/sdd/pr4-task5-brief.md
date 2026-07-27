## Task 5: 删除 EngineShared，字段合并到 EngineContext 顶层

**Files:**
- Modify: `src/crawl/engine.rs:32-92`（删除 EngineShared，字段挪到 EngineContext）
- Modify: `src/crawl/engine.rs:97-551`（所有 ctx.shared.xxx → ctx.xxx）
- Modify: `src/crawl/engine.rs:893-1438`（4 个测试辅助函数更新）
- Modify: `src/crawl/runner.rs:268-321`（run_inner 创建 EngineContext 改为顶层字段）
- Modify: `src/crawl/runner.rs:323-532`（run_inner 中 ctx.shared.xxx → ctx.xxx）

**Interfaces:**
- Consumes: Task 4 的 `EngineContext`
- Produces: `EngineContext` 直接持有 sched/follow_tx/proxy_clients/control/work_notify/middleware_chain/rule_engine/cf_domain_locks（不再有 shared 子结构）

- [ ] **Step 1: 写失败的测试**

在 `src/crawl/engine.rs` 的 tests 模块中追加测试：

```rust
    /// PR4 Task 5：验证 EngineContext 直接持有 sched/control 等字段（无 shared 子结构）。
    #[test]
    fn test_engine_context_no_shared_substruct() {
        let (ctx, _stats) = make_ctx();
        // 直接访问 ctx.sched（原 ctx.shared.sched）
        let _sched: &Arc<scheduler::Scheduler> = &ctx.sched;
        // 直接访问 ctx.control
        let _control: &Arc<control::EngineControl> = &ctx.control;
        // 直接访问 ctx.work_notify
        let _notify: &Arc<tokio::sync::Notify> = &ctx.work_notify;
        // 直接访问 ctx.middleware_chain
        let _chain: &Arc<middleware::MiddlewareChain> = &ctx.middleware_chain;
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib test_engine_context_no_shared_substruct`
Expected: FAIL（`ctx.sched` 字段不存在，仍为 `ctx.shared.sched`）

- [ ] **Step 3: 写最小实现**

**3a. 替换 engine.rs 第 32-92 行（删除 EngineShared，字段挪到 EngineContext）：**

```rust
// === EngineContext: 打包所有共享状态 ===

/// Engine 运行时上下文（单 Spider）。
///
/// PR4 重构：删除 EngineShared 子结构，字段直接展开到 EngineContext 顶层。
/// 分为三组：
/// - 只读配置：config (Arc<runner::EngineConfig>) + client (Arc<FetchClient>)
/// - 跨 task 共享可变状态：sched / follow_tx / proxy_clients / control / work_notify / middleware_chain / rule_engine / cf_domain_locks
/// - per-run 可变状态：state
pub(crate) struct EngineContext {
    // === 只读配置 ===
    pub config: Arc<crate::crawl::runner::EngineConfig>,
    pub client: Arc<crate::fetcher::FetchClient>,

    // === 跨 task 共享可变状态（原 EngineShared） ===
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

    // === per-run 可变状态 ===
    pub state: EngineState,
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

**3b. 全局替换 ctx.shared → ctx（在 engine.rs 中）：**

所有以下访问需替换：
- `ctx.shared.sched` → `ctx.sched`
- `ctx.shared.follow_tx` → `ctx.follow_tx`
- `ctx.shared.proxy_clients` → `ctx.proxy_clients`
- `ctx.shared.control` → `ctx.control`
- `ctx.shared.work_notify` → `ctx.work_notify`
- `ctx.shared.middleware_chain` → `ctx.middleware_chain`
- `ctx.shared.rule_engine` → `ctx.rule_engine`
- `ctx.shared.cf_domain_locks` → `ctx.cf_domain_locks`

涉及行（engine.rs）：
- 第 99-107: `ctx.shared.control` → `ctx.control`（check_control_and_hook）
- 第 142-164: `ctx.shared.middleware_chain` → `ctx.middleware_chain`（process_request）
- 第 203-265: `ctx.shared.middleware_chain` → `ctx.middleware_chain`（process_response）
- 第 222-236: `ctx.config.max_refetch_rounds` 不变
- 第 246: `ctx.shared.rule_engine` / `ctx.shared.proxy_clients` / `ctx.shared.cf_domain_locks` → `ctx.rule_engine` / `ctx.proxy_clients` / `ctx.cf_domain_locks`
- 第 335-340: `ctx.shared.follow_tx` / `ctx.shared.work_notify` → `ctx.follow_tx` / `ctx.work_notify`
- 第 343-351: `ctx.state.tx` / `ctx.state.stats` 不变
- 第 372-493: `ctx.shared.middleware_chain` / `ctx.shared.rule_engine` / `ctx.shared.proxy_clients` / `ctx.shared.cf_domain_locks` / `ctx.config.max_retries` → 对应 `ctx.xxx`
- 第 410-417: `ctx.shared.rule_engine` → `ctx.rule_engine`
- 第 444: `ctx.state.spider` 不变
- 第 463-470: `ctx.state.tx` 不变

**3c. 更新 InFlightGuard 创建（engine.rs:441-448，runner.rs 中）：**

```rust
                        let ctx_c = ctx.clone();
                        let fut = async move {
                            let _g1 = engine::InFlightGuard {
                                counter: ctx_c.state.global_in_flight.clone(),
                                work_notify: ctx_c.work_notify.clone(),  // 原 ctx_c.shared.work_notify
                            };
                            let _g2 = engine::InFlightGuard {
                                counter: ctx_c.state.stats.in_flight.clone(),
                                work_notify: ctx_c.work_notify.clone(),  // 原 ctx_c.shared.work_notify
                            };
                            // ... 不变
                        };
```

**3d. 更新 runner.rs run_inner 创建 EngineContext（runner.rs:268-321）：**

```rust
        let ctx = Arc::new(engine::EngineContext {
            config: Arc::clone(self.config()),
            client: fetch_client.clone(),
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

**3e. 更新 runner.rs run_inner 中 ctx.shared 访问（runner.rs:323-532）：**

- 第 324: `ctx.shared.middleware_chain` → `ctx.middleware_chain`
- 第 326-330: `ctx.shared.middleware_chain` → `ctx.middleware_chain`
- 第 337: `ctx.shared.work_notify` → `ctx.work_notify`（autoscale set_work_notify）
- 第 363-373: `ctx.shared.control` / `ctx.shared.sched` → `ctx.control` / `ctx.sched`
- 第 388: `ctx.shared.sched` → `ctx.sched`
- 第 417-424: `ctx.shared.work_notify` → `ctx.work_notify`
- 第 426-433: `ctx.shared.sched` / `ctx.shared.work_notify` → `ctx.sched` / `ctx.work_notify`
- 第 442-448: `ctx_c.shared.work_notify` → `ctx_c.work_notify`（InFlightGuard）
- 第 475: `ctx.state.stats` 不变

**3f. 更新 4 个测试辅助函数（engine.rs:917-953, 1029-1073, 1139-1187, 1386-1438）：**

每个 make_ctx / make_ctx_with_retry / make_ctx_auto / make_ctx_with_tx 中，原 `shared: EngineShared { ... }` 改为直接展开到 EngineContext 顶层：

```rust
        let ctx = EngineContext {
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
            // 原 EngineShared 字段直接展开
            sched: Arc::new(scheduler::Scheduler::new()),
            follow_tx,
            proxy_clients: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
            control: Arc::new(control::EngineControl::new()),
            work_notify: Arc::new(tokio::sync::Notify::new()),
            middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
            cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build()),
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
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib test_engine_context_no_shared_substruct`
Expected: PASS

Run: `cargo build --lib`
Expected: 编译成功

Run: `cargo test --lib`
Expected: 现有测试全绿

- [ ] **Step 5: 提交**

```bash
git add src/crawl/engine.rs src/crawl/runner.rs
git commit -m "refactor: 删除 EngineShared，字段合并到 EngineContext 顶层"
```

---

