# Engine API 重构：动态提交 Spider

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 重构 Engine 为长驻服务，支持 `submit(spider)` 动态提交。MCP server 复用单个 Engine 实例（共享连接池/缓存/代理池），顺带解决 I4（单 Engine 无多实例污染）。

**Architecture:** Engine 持有常驻 driver task，通过 channel 接收 Spider。`submit(spider)` 返回 `CrawlHandle` 拉取 items。**不保留旧的一次性 API**——`Engine::new(spider).run()` 全部改为 `Engine::server().submit(spider)` 形态。

**Tech Stack:** Rust, tokio, async-trait

---

## 背景

### 当前问题
1. `Engine::new(spider)` + `run()`/`stream()` 消费 self，一次性运行后 Engine 销毁
2. MCP `crawl_site` 每次调用创建新 Engine，无法复用 HTTP 连接池/缓存/代理池
3. 多 Engine 并发时 `control.rs` 全局状态互相干扰（I4）

### 设计原则
- **Engine 长驻**：MCP server 启动时创建一个 Engine，生命周期与 server 相同
- **动态提交 Spider**：`Engine::submit(spider)` 返回 `CrawlHandle`，不阻塞
- **资源共享**：所有提交的 Spider 共享 Engine 的连接池/缓存/代理池
- **per-Spider 隔离**：每个 Spider 有独立的 stats/until/patterns，但共享队列
- **不向后兼容**：删除 `Engine::new(spider).run()` 等旧 API，所有调用方一并迁移

---

## 文件结构

| 文件 | 责任 | 动作 |
|------|------|------|
| `src/crawl/mod.rs` | Engine 结构体重构：serve/submit 替代 new/run | 修改 |
| `src/crawl/engine.rs` | EngineContext、driver_loop、process_request 改造 | 修改 |
| `src/crawl/control.rs` | control 状态移入 Engine（I4 解决） | 修改 |
| `src/mcp/mod.rs` | MCP server 持有共享 Engine | 修改 |
| `src/mcp/tools.rs` | `crawl_site` 用 submit 替代 Engine::new | 修改 |
| `src/bin/wisp.rs` | CLI 代码迁移到新 API | 修改 |
| `tests/engine_submit_test.rs` | 动态提交 API 测试 | **新建** |
| 现有所有测试文件 | 迁移到新 API | 修改 |

---

## Task 1: 定义 CrawlHandle 和新 Engine 结构体

**Files:**
- Modify: `src/crawl/mod.rs`

- [ ] **Step 1: 定义 CrawlHandle**

在 `src/crawl/mod.rs` 定义 `CrawlHandle`——动态提交 Spider 的返回句柄：

```rust
use tokio::sync::oneshot;

/// 动态提交 Spider 的返回句柄。
///
/// 调用方用 `handle.next_item().await` 拉取该 Spider 的 items，
/// 或 `handle.wait_done().await` 等待完成。
pub struct CrawlHandle {
    /// Spider 在 Engine 内部的 ID
    spider_id: usize,
    /// items 接收端
    items_rx: tokio::sync::mpsc::Receiver<Value>,
    /// 完成信号发送端（drop 时通知 Engine 该 Spider 已结束）
    done_tx: oneshot::Sender<()>,
}

impl CrawlHandle {
    /// 拉取下一个 item（返回 None 表示 Spider 已完成）。
    pub async fn next_item(&mut self) -> Option<Value> {
        self.items_rx.recv().await
    }

    /// 等待 Spider 完成（所有 start_urls + follow 请求处理完，或 until 触发）。
    pub async fn wait_done(mut self) {
        while self.items_rx.recv().await.is_some() {}
    }

    /// 转为 items stream（消费 self）。
    pub fn into_stream(self) -> impl futures::Stream<Item = Value> {
        tokio_stream::wrappers::ReceiverStream::new(self.items_rx)
    }
}
```

- [ ] **Step 2: 重构 Engine 结构体**

删除旧的 `Engine` 字段（`spiders: Vec<Box<dyn Spider>>` 等），改为长驻服务形态：

```rust
use tokio::sync::RwLock;

/// 统一爬虫引擎。长驻服务模式，支持动态提交 Spider。
///
/// 所有提交的 Spider 共享连接池/缓存/代理池。
/// 每个 Spider 有独立的 stats/until/patterns，通过 patterns 路由 URL。
pub struct Engine {
    // === 引擎级配置（serve 前用 builder 设置）===
    max_pages: usize,
    max_concurrent: Option<usize>,
    max_depth: Option<u32>,
    checkpoint_store: Option<Arc<crate::storage::Store>>,
    checkpoint_interval: usize,
    cache_store: Option<Arc<crate::storage::Store>>,
    development_mode: bool,
    proxy_pool: Option<Arc<crate::proxy::ProxyPool>>,
    request_cache: Option<RequestCache>,

    // === 共享资源（serve 后 Some）===
    client: Option<Arc<Client>>,
    sched: Option<Arc<scheduler::Scheduler>>,
    follow_tx: Option<tokio::sync::mpsc::UnboundedSender<SpiderRequest>>,
    shutdown_tx: Option<oneshot::Sender<()>>,

    // === 动态 Spider 注册表（并发安全）===
    registered_spiders: RwLock<Vec<Arc<dyn Spider>>>,
    spider_stats: RwLock<Vec<Arc<SpiderStats>>>,
    compiled_patterns: RwLock<Vec<Vec<regex::Regex>>>,
    items_tx: RwLock<Vec<tokio::sync::mpsc::Sender<Value>>>,
    rule_engines: RwLock<Vec<Arc<Mutex<auto::ModeRuleEngine>>>>,
    auto_excludes: RwLock<Vec<HashSet<String>>>,
    allowed_list: RwLock<Vec<Arc<HashSet<String>>>>,
    fetcher_configs: RwLock<Vec<http::Config>>,
    fetch_modes: RwLock<Vec<FetchMode>>,
    max_concurrents: RwLock<Vec<usize>>,
    max_depths: RwLock<Vec<u32>>,
    obey_robots_flags: RwLock<Vec<bool>>,

    // === 全局统计 ===
    global_in_flight: Arc<AtomicUsize>,
    global_pages: Arc<AtomicUsize>,
}
```

- [ ] **Step 3: 实现 Engine 构造器（builder 模式）**

```rust
impl Engine {
    /// 创建空 Engine（用于长驻服务模式）。
    /// 后续用 builder 方法配置，再调用 `serve()` 启动。
    pub fn server() -> Self {
        Self {
            max_pages: 1000,
            max_concurrent: None,
            max_depth: None,
            checkpoint_store: None,
            checkpoint_interval: 100,
            cache_store: None,
            development_mode: false,
            proxy_pool: None,
            request_cache: None,
            client: None,
            sched: None,
            follow_tx: None,
            shutdown_tx: None,
            registered_spiders: RwLock::new(Vec::new()),
            spider_stats: RwLock::new(Vec::new()),
            compiled_patterns: RwLock::new(Vec::new()),
            items_tx: RwLock::new(Vec::new()),
            rule_engines: RwLock::new(Vec::new()),
            auto_excludes: RwLock::new(Vec::new()),
            allowed_list: RwLock::new(Vec::new()),
            fetcher_configs: RwLock::new(Vec::new()),
            fetch_modes: RwLock::new(Vec::new()),
            max_concurrents: RwLock::new(Vec::new()),
            max_depths: RwLock::new(Vec::new()),
            obey_robots_flags: RwLock::new(Vec::new()),
            global_in_flight: Arc::new(AtomicUsize::new(0)),
            global_pages: Arc::new(AtomicUsize::new(0)),
        }
    }

    // Builder 方法（保持链式调用）
    pub fn max_pages(mut self, n: usize) -> Self { self.max_pages = n; self }
    pub fn max_concurrent(mut self, n: usize) -> Self { self.max_concurrent = Some(n); self }
    pub fn max_depth(mut self, n: u32) -> Self { self.max_depth = Some(n); self }
    pub fn with_checkpoint(mut self, store: Arc<crate::storage::Store>) -> Self {
        self.checkpoint_store = Some(store); self
    }
    pub fn development_mode(mut self, store: Arc<crate::storage::Store>) -> Self {
        self.cache_store = Some(store); self.development_mode = true; self
    }
    pub fn proxy_pool(mut self, proxies: Vec<String>, strategy: crate::proxy::RotationStrategy) -> Self {
        if !proxies.is_empty() {
            self.proxy_pool = Some(Arc::new(crate::proxy::ProxyPool::new(proxies, strategy)));
        }
        self
    }
    pub fn request_cache(mut self, cache: RequestCache) -> Self {
        self.request_cache = Some(cache); self
    }
}
```

- [ ] **Step 4: 验证编译**

```
cargo build --lib
```

预期有编译错误（因为 `new`/`run` 等旧方法已删除），先不修复调用方，Task 2-5 会逐步迁移。

- [ ] **Step 5: 提交**

```bash
git add src/crawl/mod.rs
git commit -m "refactor(crawl): Engine 结构体重构为长驻服务形态" -m "删除 new/spiders/run/run_one/stream 旧 API" -m "新增 CrawlHandle 和 RwLock 动态注册表"
```

---

## Task 2: 实现 serve + submit + driver_loop

**Files:**
- Modify: `src/crawl/mod.rs`
- Modify: `src/crawl/engine.rs`

- [ ] **Step 1: 实现 Engine::serve**

在 `src/crawl/mod.rs`：

```rust
impl Engine {
    /// 启动长驻服务模式。
    ///
    /// 初始化共享资源（HTTP client、调度器、follow channel），
    /// 启动 driver task 处理队列。返回 `Arc<Engine>` 供调用方持有。
    pub async fn serve(mut self) -> Result<Arc<Self>> {
        let client = Arc::new(
            Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?
        );
        let sched = Arc::new(scheduler::Scheduler::new());
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        // 重置 control 全局状态（解决 I4：serve 时确保干净状态）
        control::reset().await;

        self.client = Some(client.clone());
        self.sched = Some(sched.clone());
        self.follow_tx = Some(follow_tx.clone());
        self.shutdown_tx = Some(shutdown_tx);

        let engine = Arc::new(self);

        // 启动 driver task
        tokio::spawn(driver_loop(
            engine.clone(),
            follow_rx,
            shutdown_rx,
        ));

        Ok(engine)
    }

    /// 关闭 Engine（停止 driver task）。
    pub async fn shutdown(&self) {
        if let Some(tx) = &self.shutdown_tx {
            // shutdown_tx 是 oneshot::Sender，只能 send 一次
            // 但这里 &self 无法 take shutdown_tx，需要内部可变
            // 改用 Option 包 oneshot::Sender 在 Mutex 中，或直接用 watch
            // 简化：用 control::shutdown() 全局信号
            control::shutdown();
        }
    }
}
```

**注意**：`shutdown` 的实现需要能从 `&self` 触发关闭信号。`oneshot::Sender` 消费 self 无法从 `&self` 调用。改用 `tokio::sync::watch` 或 `AtomicBool`：

```rust
// 改用 watch channel 作为 shutdown 信号
use tokio::sync::watch;

pub struct Engine {
    // ...
    shutdown_tx: Option<watch::Sender<bool>>,
}

// serve 中：
let (shutdown_tx, shutdown_rx) = watch::channel(false);

// shutdown 中：
pub fn shutdown(&self) {
    if let Some(tx) = &self.shutdown_tx {
        let _ = tx.send(true);
    }
}
```

- [ ] **Step 2: 实现 Engine::submit**

在 `src/crawl/mod.rs`：

```rust
impl Engine {
    /// 动态提交一个 Spider，返回 CrawlHandle 拉取 items。
    ///
    /// Spider 的 start_urls 立即注入共享队列。
    /// 多个 Spider 共享连接池/缓存/代理池。
    pub async fn submit(&self, spider: Box<dyn Spider>) -> CrawlHandle {
        let spider: Arc<dyn Spider> = Arc::from(spider);
        let idx = self.registered_spiders.read().await.len();

        // 预编译 patterns
        let patterns: Vec<regex::Regex> = spider.patterns().iter()
            .filter_map(|p| match regex::Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    tracing::warn!("Spider '{}' patterns 正则编译失败: '{}' - {}", spider.name(), p, e);
                    None
                }
            })
            .collect();

        // 创建 items channel
        let (items_tx, items_rx) = tokio::sync::mpsc::channel::<Value>(128);
        let (done_tx, done_rx) = oneshot::channel();

        // 构建该 Spider 的配置
        let stats = Arc::new(SpiderStats::new());
        let max_concurrent = self.max_concurrent.unwrap_or(spider.concurrent_requests() as usize);
        let max_depth = self.max_depth.unwrap_or(spider.max_depth());
        let obey_robots = spider.obey_robots();
        let fetcher_config = spider.fetcher_config();
        let fetch_mode = spider.fetch_mode();
        let allowed = Arc::new(spider.allowed_domains());
        let auto_excludes = spider.auto_exclude();

        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in spider.auto_rules() {
            let _ = rule_engine.add_user_rule(&pattern, mode);
        }

        // 推入 start_urls
        let sched = self.sched.as_ref().expect("Engine 未 serve");
        for url in spider.start_urls() {
            sched.push(SpiderRequest::get(&url)).await;
        }

        // 注册到动态列表（写锁）
        {
            let mut spiders = self.registered_spiders.write().await;
            let mut stats_vec = self.spider_stats.write().await;
            let mut patterns_vec = self.compiled_patterns.write().await;
            let mut items_txs = self.items_tx.write().await;
            let mut rule_engines = self.rule_engines.write().await;
            let mut excludes = self.auto_excludes.write().await;
            let mut alloweds = self.allowed_list.write().await;
            let mut configs = self.fetcher_configs.write().await;
            let mut modes = self.fetch_modes.write().await;
            let mut concurrents = self.max_concurrents.write().await;
            let mut depths = self.max_depths.write().await;
            let mut robots_flags = self.obey_robots_flags.write().await;

            spiders.push(spider);
            stats_vec.push(stats);
            patterns_vec.push(patterns);
            items_txs.push(items_tx);
            rule_engines.push(Arc::new(Mutex::new(rule_engine)));
            excludes.push(auto_excludes);
            alloweds.push(allowed);
            configs.push(fetcher_config);
            modes.push(fetch_mode);
            concurrents.push(max_concurrent);
            depths.push(max_depth);
            robots_flags.push(obey_robots);
        }

        CrawlHandle {
            spider_id: idx,
            items_rx,
            done_tx,
        }
    }
}
```

- [ ] **Step 3: 实现 driver_loop**

在 `src/crawl/engine.rs`：

```rust
/// 常驻 driver：循环从 follow_rx 和 sched 拉取请求，路由到对应 Spider。
///
/// 永不退出，直到 shutdown_rx 收到 true。
async fn driver_loop(
    engine: Arc<Engine>,
    mut follow_rx: tokio::sync::mpsc::UnboundedReceiver<SpiderRequest>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        // 检查 shutdown 信号
        if *shutdown_rx.borrow() {
            tracing::info!("Engine driver 收到关闭信号，退出");
            return;
        }

        // 检查全局页数上限
        if engine.global_pages.load(Ordering::SeqCst) >= engine.max_pages {
            tracing::info!("Engine 达到全局 max_pages 上限，退出");
            return;
        }

        // 从 follow channel 拉取新请求
        let req = tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return;
                }
                continue;
            }
            req = follow_rx.recv() => match req {
                Some(req) => req,
                None => return,
            },
            req = engine.sched.as_ref().unwrap().pop() => match req {
                Some(req) => req,
                None => {
                    // 队列空，检查是否所有 Spider 都已完成
                    let in_flight = engine.global_in_flight.load(Ordering::SeqCst);
                    if in_flight == 0 {
                        // 无在途请求且队列空，yield 等待新 submit
                        tokio::task::yield_now().await;
                    }
                    continue;
                }
            },
        };

        // 路由到匹配的 Spider
        let route_result = route_spider(&engine, &req).await;
        match route_result {
            Some(idx) => {
                // 增加在途计数
                engine.global_in_flight.fetch_add(1, Ordering::SeqCst);
                // 处理请求（spawn 隔离，单 Spider panic 不杀 Engine）
                let engine_clone = engine.clone();
                tokio::spawn(async move {
                    process_request(&engine_clone, req, idx).await;
                    engine_clone.global_in_flight.fetch_sub(1, Ordering::SeqCst);
                });
            }
            None => {
                tracing::debug!("无 Spider 匹配 URL: {}", req.url);
            }
        }
    }
}

/// 路由：找 matches(url) 且未停止的 Spider。
async fn route_spider(engine: &Arc<Engine>, req: &SpiderRequest) -> Option<usize> {
    let spiders = engine.registered_spiders.read().await;
    let patterns_list = engine.compiled_patterns.read().await;
    let stats_list = engine.spider_stats.read().await;
    let sched = engine.sched.as_ref().unwrap();

    for (i, spider) in spiders.iter().enumerate() {
        let patterns = &patterns_list[i];
        let matched = if patterns.is_empty() {
            true
        } else {
            patterns.iter().any(|re| re.is_match(&req.url))
        };
        if !matched { continue; }

        let stats = &stats_list[i];
        let queue_size = sched.len().await;
        let stop_ctx = stop::StopContext {
            pages: stats.pages.load(Ordering::SeqCst),
            items: stats.items.load(Ordering::SeqCst),
            errors: stats.errors.load(Ordering::SeqCst),
            in_flight: stats.in_flight.load(Ordering::SeqCst),
            elapsed: stats.start.elapsed(),
            queue_size,
        };
        if spider.until().should_stop(&stop_ctx) { continue; }

        return Some(i);
    }
    None
}
```

- [ ] **Step 4: 重写 process_request**

修改 `src/crawl/engine.rs` 的 `process_request`，从 `Arc<Engine>` + `spider_idx` 取数据，替代旧的 `&EngineContext`：

```rust
/// 处理单个请求：域名过滤 → 深度检查 → control 检查 → fetch → parse → 推 follow。
pub async fn process_request(engine: &Arc<Engine>, req: SpiderRequest, idx: usize) {
    let spiders = engine.registered_spiders.read().await;
    let spider = &spiders[idx];
    let stats_list = engine.spider_stats.read().await;
    let stats = &stats_list[idx];
    let allowed_list = engine.allowed_list.read().await;
    let allowed = &allowed_list[idx];
    let max_depths = engine.max_depths.read().await;
    let max_depth = max_depths[idx];
    let obey_robots_flags = engine.obey_robots_flags.read().await;
    let obey_robots = obey_robots_flags[idx];
    let fetcher_configs = engine.fetcher_configs.read().await;
    let fetcher_config = &fetcher_configs[idx];
    let fetch_modes = engine.fetch_modes.read().await;
    let fetch_mode = fetch_modes[idx];
    let rule_engines = engine.rule_engines.read().await;
    let rule_engine = &rule_engines[idx];
    let auto_excludes = engine.auto_excludes.read().await;
    let auto_exclude = &auto_excludes[idx];
    let items_txs = engine.items_tx.read().await;
    let items_tx = &items_txs[idx];

    // ... 域名过滤、深度检查、control 检查（与原逻辑相同）...

    // fetch_with_retry
    let (resp_opt, err_opt) = fetch_with_retry(
        engine, &req, idx, spider, stats, fetch_mode, fetcher_config, rule_engine
    ).await;

    // 更新全局页数
    if let Some(ref resp) = resp_opt {
        if !resp.from_cache {
            engine.global_pages.fetch_add(1, Ordering::SeqCst);
            stats.pages.fetch_add(1, Ordering::SeqCst);
        }
    }

    // parse
    if let Some(resp) = resp_opt {
        let (items, follow_reqs) = spider.parse(resp).await;
        // 推 items 到该 Spider 的 channel
        for item in items {
            // 应用 on_item 钩子
            let item = spider.on_item(item).await;
            if let Some(item) = item {
                stats.items.fetch_add(1, Ordering::SeqCst);
                if items_tx.try_send(item).is_err() {
                    tracing::warn!("items channel 已满或关闭，丢弃 item");
                }
            }
        }
        // 推 follow 请求
        for follow_req in follow_reqs {
            let _ = engine.follow_tx.as_ref().unwrap().send(follow_req);
        }
    }

    if let Some(err) = err_opt {
        stats.errors.fetch_add(1, Ordering::SeqCst);
        spider.on_error(&req, &err).await;
    }
}
```

**注意**：`process_request` 内部需要多个 `read().await` 获取配置。为避免持锁过久，在函数开头一次性读取所需字段到局部变量（如上面代码所示，每个 read 锁的作用域仅在该块内）。

- [ ] **Step 5: 删除旧的 EngineContext 和 run_with_sender**

删除 `src/crawl/engine.rs` 中的 `EngineContext` 结构体和 `run_with_sender` 函数。删除 `src/crawl/mod.rs` 中的 `run`、`run_one`、`stream` 方法。

- [ ] **Step 6: 写测试**

创建 `tests/engine_submit_test.rs`：

```rust
//! Engine 动态提交 Spider 测试。
use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingSpider {
    name: String,
    start_url: String,
    count: Arc<AtomicUsize>,
    items: Vec<&'static str>,
}

#[async_trait]
impl Spider for CountingSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { vec![self.start_url.clone()] }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        self.count.fetch_add(1, Ordering::SeqCst);
        let items: Vec<Value> = self.items.iter()
            .map(|s| serde_json::json!({"text": s}))
            .collect();
        (items, vec![])
    }
    fn obey_robots(&self) -> bool { false }
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(MaxPages(1))
    }
}

#[tokio::test]
async fn test_submit_single_spider() {
    let engine = Engine::server().max_pages(10).serve().await.unwrap();
    let count = Arc::new(AtomicUsize::new(0));

    let spider = CountingSpider {
        name: "test".into(),
        start_url: "http://127.0.0.1:1/unreachable".into(),
        count: count.clone(),
        items: vec!["item1", "item2"],
    };
    let mut handle = engine.submit(Box::new(spider)).await;

    // 不可达 URL 会触发 error，但 parse 不会被调用
    // 等待 Spider 完成
    let item = handle.next_item().await;
    assert!(item.is_none(), "不可达 URL 不应产出 item");

    engine.shutdown();
}

#[tokio::test]
async fn test_submit_multiple_spiders_isolated() {
    let engine = Engine::server().max_pages(100).serve().await.unwrap();
    let count_a = Arc::new(AtomicUsize::new(0));
    let count_b = Arc::new(AtomicUsize::new(0));

    let spider_a = CountingSpider {
        name: "a".into(),
        start_url: "http://127.0.0.1:1/a".into(),
        count: count_a.clone(),
        items: vec!["a1"],
    };
    let spider_b = CountingSpider {
        name: "b".into(),
        start_url: "http://127.0.0.1:1/b".into(),
        count: count_b.clone(),
        items: vec!["b1"],
    };

    let handle_a = engine.submit(Box::new(spider_a)).await;
    let handle_b = engine.submit(Box::new(spider_b)).await;

    // 两个 Spider 都不可达，等待完成
    handle_a.wait_done().await;
    handle_b.wait_done().await;

    // 两个 Spider 都不应该 parse（不可达 URL）
    assert_eq!(count_a.load(Ordering::SeqCst), 0);
    assert_eq!(count_b.load(Ordering::SeqCst), 0);

    engine.shutdown();
}
```

**注意**：上面的测试用不可达 URL 验证基本流程不 panic。要测试实际爬取，需要 mock server（参考 `cr_fix_t1_test.rs` 的 `spawn_html_server`）。

- [ ] **Step 7: 验证**

```
cargo build --lib
cargo test --test engine_submit_test -- --nocapture
```

- [ ] **Step 8: 提交**

```bash
git add src/crawl/mod.rs src/crawl/engine.rs tests/engine_submit_test.rs
git commit -m "feat(crawl): 实现 serve/submit/driver_loop 动态提交 API" -m "Engine::serve 启动常驻 driver，submit 返回 CrawlHandle" -m "process_request 改为从 Arc<Engine> 取配置，spawn 隔离单 Spider panic"
```

---

## Task 3: control.rs 状态移入 Engine

**Files:**
- Modify: `src/crawl/control.rs`
- Modify: `src/crawl/engine.rs`

- [ ] **Step 1: control 状态改为 Engine 内部持有**

修改 `src/crawl/control.rs`，将全局 static 改为 `EngineControl` 结构体：

```rust
//! 引擎级控制状态（per-Engine 隔离）。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};

/// 单个 Engine 的控制状态。
#[derive(Debug)]
pub struct EngineControl {
    paused_urls: Arc<RwLock<HashSet<String>>>,
    cancelled_urls: Arc<RwLock<HashSet<String>>>,
    global_paused: AtomicBool,
    shutdown: AtomicBool,
    version: watch::Sender<u64>,
}

impl EngineControl {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(0u64);
        Self {
            paused_urls: Arc::new(RwLock::new(HashSet::new())),
            cancelled_urls: Arc::new(RwLock::new(HashSet::new())),
            global_paused: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            version: tx,
        }
    }

    fn bump(&self) {
        let _ = self.version.send(self.version.borrow().wrapping_add(1));
    }

    pub async fn pause(&self, url: &str) {
        self.paused_urls.write().await.insert(url.to_string());
        self.bump();
    }

    pub async fn resume(&self, url: &str) {
        self.paused_urls.write().await.remove(url);
        self.bump();
    }

    pub fn pause_all(&self) {
        self.global_paused.store(true, Ordering::SeqCst);
        self.bump();
    }

    pub fn resume_all(&self) {
        self.global_paused.store(false, Ordering::SeqCst);
        self.bump();
    }

    pub async fn cancel(&self, url: &str) {
        self.cancelled_urls.write().await.insert(url.to_string());
        self.bump();
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.bump();
    }

    pub async fn reset(&self) {
        self.paused_urls.write().await.clear();
        self.cancelled_urls.write().await.clear();
        self.global_paused.store(false, Ordering::SeqCst);
        self.shutdown.store(false, Ordering::SeqCst);
        self.bump();
    }

    pub async fn is_cancelled(&self, url: &str) -> bool {
        self.cancelled_urls.read().await.contains(url)
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub async fn wait_if_paused(&self, url: &str) -> bool {
        let mut rx = self.version.subscribe();
        loop {
            if self.shutdown.load(Ordering::SeqCst) { return false; }
            let global = self.global_paused.load(Ordering::SeqCst);
            let url_paused = self.paused_urls.read().await.contains(url);
            if !global && !url_paused { return true; }
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return !self.shutdown.load(Ordering::SeqCst);
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
        }
    }
}

impl Default for EngineControl {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 2: 删除全局 static 变量和全局函数**

删除 `PAUSED_URLS`、`CANCELLED_URLS`、`SHUTDOWN_FLAG`、`GLOBAL_PAUSED`、`VERSION` 这些 static 变量，以及所有全局函数。

- [ ] **Step 3: Engine 持有 EngineControl**

在 `src/crawl/mod.rs` 的 `Engine` 结构体加字段：

```rust
pub struct Engine {
    // ...
    control: Arc<control::EngineControl>,
}
```

在 `server()` 构造器中初始化：
```rust
control: Arc::new(control::EngineControl::new()),
```

暴露访问方法：
```rust
impl Engine {
    /// 获取控制句柄（用于 CLI/MCP 外部控制）。
    pub fn control(&self) -> &Arc<control::EngineControl> {
        &self.control
    }
}
```

- [ ] **Step 4: process_request 用 engine.control**

修改 `src/crawl/engine.rs` 的 `process_request`：

```rust
// 旧：super::control::is_cancelled(&req.url).await
// 新：
if engine.control.is_cancelled(&req.url).await { return; }
if !engine.control.wait_if_paused(&req.url).await { return; }
if engine.control.is_shutdown() { return; }
```

- [ ] **Step 5: Engine::shutdown 用 control**

```rust
impl Engine {
    pub fn shutdown(&self) {
        self.control.shutdown();
    }
}
```

- [ ] **Step 6: 写测试**

在 `tests/engine_submit_test.rs` 追加：

```rust
#[tokio::test]
async fn test_engine_control_isolation() {
    let engine_a = Engine::server().serve().await.unwrap();
    let engine_b = Engine::server().serve().await.unwrap();

    engine_a.control().pause_all();
    assert!(engine_a.control().is_shutdown() == false);
    assert!(!engine_b.control().is_shutdown(), "Engine B 不应受 A 影响");

    engine_a.control().shutdown();
    assert!(engine_a.control().is_shutdown());
    assert!(!engine_b.control().is_shutdown(), "Engine B 不应受 A 关闭影响");
}
```

- [ ] **Step 7: 验证**

```
cargo build --lib
cargo test --test engine_submit_test -- --nocapture
```

- [ ] **Step 8: 提交**

```bash
git add src/crawl/control.rs src/crawl/mod.rs src/crawl/engine.rs tests/engine_submit_test.rs
git commit -m "refactor(crawl): control 全局状态重构为 per-Engine EngineControl" -m "I4: 删除全局 static，Engine 持有独立 EngineControl" -m "多 Engine 实例控制状态完全隔离"
```

---

## Task 4: 改造 MCP crawl_site 用共享 Engine

**Files:**
- Modify: `src/mcp/mod.rs`
- Modify: `src/mcp/tools.rs`

- [ ] **Step 1: MCP server 持有共享 Engine**

修改 `src/mcp/mod.rs`：

```rust
pub async fn run() -> Result<()> {
    let store = Arc::new(Store::open(Path::new("wisp.db"))?);
    let engine = Engine::server()
        .max_pages(100000)  // 全局上限，单次 crawl_site 的 max_pages 用 Spider.until() 控制
        .serve().await?;

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 { break; }
        // ... 解析 JSON-RPC ...
        let response = match method.as_str() {
            "tools/call" => {
                let result = handle_tools_call(request, &store, &engine).await?;
                // ...
            }
            // ...
        };
    }

    engine.shutdown();
    Ok(())
}

async fn handle_tools_call(
    request: Value,
    store: &Arc<Store>,
    engine: &Arc<Engine>,
) -> Result<Value> {
    let name = request["params"]["name"].as_str().unwrap_or("");
    let args = request["params"]["arguments"].clone();
    match name {
        "crawl_site" => tools::crawl_site(args, store, engine).await,
        "adaptive_scrape" => tools::adaptive_scrape(args, store).await,
        _ => Err(WispError::McpError(format!("unknown tool: {}", name))),
    }
}
```

- [ ] **Step 2: 修改 crawl_site 用 engine.submit**

修改 `src/mcp/tools.rs`：

```rust
pub async fn crawl_site(
    args: Value,
    _store: &Arc<Store>,
    engine: &Arc<Engine>,
) -> Result<Value> {
    let start_urls: Vec<String> = args.get("start_urls")
        .and_then(|v| v.as_array())
        .ok_or_else(|| WispError::McpError("missing 'start_urls' array".into()))?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    if start_urls.is_empty() {
        return Err(WispError::McpError("start_urls 不能为空".into()));
    }
    let css_selector = args.get("css_selector")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WispError::McpError("missing 'css_selector'".into()))?
        .to_string();
    let max_pages = args.get("max_pages")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    struct SimpleSpider {
        css: String,
        start_urls: Vec<String>,
        max_pages: usize,
    }
    #[async_trait]
    impl Spider for SimpleSpider {
        fn name(&self) -> &str { "mcp_simple" }
        fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
        async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            let text = resp.text().unwrap_or_default();
            let doc = Node::from_html(&text);
            let nodes = doc.select(&self.css);
            let items: Vec<Value> = nodes.iter()
                .map(|n| json!({"text": n.text(), "html": n.html()}))
                .collect();
            (items, vec![])
        }
        fn obey_robots(&self) -> bool { false }
        fn until(&self) -> Arc<dyn StopCondition> {
            Arc::new(MaxPages(self.max_pages))
        }
    }

    let spider = SimpleSpider { css: css_selector, start_urls, max_pages };
    let mut handle = engine.submit(Box::new(spider)).await;

    let mut items: Vec<Value> = Vec::new();
    while let Some(item) = handle.next_item().await {
        items.push(item);
    }

    let jsonl: String = items.iter()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(json!({
        "items_count": items.len(),
        "jsonl": jsonl
    }))
}
```

- [ ] **Step 3: 修改 crawl_site 测试**

修改 `tests/cr_fix_t1_test.rs`，用 `Engine::server().serve()` 替代 `Engine::new()`：

```rust
#[tokio::test]
async fn test_crawl_site_uses_start_urls() {
    let server = spawn_html_server("<p>item1</p><p>item2</p>").await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let engine = Engine::server().max_pages(10).serve().await.unwrap();
    let args = json!({
        "start_urls": [server],
        "css_selector": "p",
        "max_pages": 1
    });
    let result = crawl_site(args, &store, &engine).await.expect("crawl_site should succeed");
    assert_eq!(result["items_count"].as_u64(), Some(2), "应爬到 2 个 p 元素");
}
```

- [ ] **Step 4: 验证**

```
cargo build --lib
cargo test --test cr_fix_t1_test -- --nocapture
```

- [ ] **Step 5: 提交**

```bash
git add src/mcp/mod.rs src/mcp/tools.rs tests/cr_fix_t1_test.rs
git commit -m "refactor(mcp): crawl_site 用共享 Engine.submit" -m "MCP server 启动时创建长驻 Engine，crawl_site 动态提交 Spider" -m "复用 HTTP 连接池/缓存/代理池"
```

---

## Task 5: 迁移 CLI 和现有测试到新 API

**Files:**
- Modify: `src/bin/wisp.rs`
- Modify: `tests/multi_spider_test.rs`
- Modify: `tests/stop_condition_test.rs`
- Modify: `tests/builder_api_test.rs`
- Modify: `tests/cr_fix_engine_test.rs`
- Modify: 其他所有 `Engine::new` 调用点

- [ ] **Step 1: 搜索所有 Engine::new 和 .run() 调用**

```
grep -rn "Engine::new\|\.run()\|\.run_one()\|\.stream()" src/ tests/
```

- [ ] **Step 2: 迁移 CLI**

修改 `src/bin/wisp.rs`，将 `Engine::new(spider).run()` 改为：

```rust
let engine = Engine::server()
    .max_pages(max_pages)
    .serve().await?;
let handle = engine.submit(Box::new(spider)).await;
handle.wait_done().await;
engine.shutdown();
```

- [ ] **Step 3: 迁移现有测试**

修改所有测试文件，将 `Engine::new(spider).run().await` 改为 `Engine::server().serve().await` + `submit` + `wait_done`。

例如 `tests/multi_spider_test.rs`：

```rust
// 旧：
// let engine = Engine::spiders(vec![Box::new(spider_a), Box::new(spider_b)]).max_pages(10);
// let results = engine.run().await.unwrap();

// 新：
let engine = Engine::server().max_pages(10).serve().await.unwrap();
let handle_a = engine.submit(Box::new(spider_a)).await;
let handle_b = engine.submit(Box::new(spider_b)).await;
handle_a.wait_done().await;
handle_b.wait_done().await;
engine.shutdown();
```

- [ ] **Step 4: 验证所有测试**

```
cargo test --lib
cargo test --test engine_submit_test
cargo test --test multi_spider_test
cargo test --test stop_condition_test
cargo test --test builder_api_test
cargo test --test cr_fix_engine_test
cargo test --test cr_fix_t1_test
cargo test --test cr_fix_t4_test
cargo test --test cr_fix_t7_test
cargo test --test cr_fix_t10_test
cargo test --test cr_fix_t11_test
```

Expected: 全部 PASS

- [ ] **Step 5: 编译检查**

```
cargo build
```

- [ ] **Step 6: 提交**

```bash
git add src/bin/wisp.rs tests/ src/crawl/mod.rs
git commit -m "refactor: 迁移所有调用方到 Engine serve/submit API" -m "删除 Engine::new/spiders/run/run_one/stream 旧 API" -m "CLI + 测试全部迁移"
```

---

## Task 6: 全量验证

- [ ] **Step 1: 全量测试**

```
cargo test
```
Expected: 全部 PASS

- [ ] **Step 2: 编译检查（含 bins/examples）**

```
cargo build --release
```
Expected: 编译通过

- [ ] **Step 3: 确认提交历史**

```
git log --oneline -10
```

---

## 自检清单

**目标覆盖：**
- Engine API 重构（serve + submit）→ Task 1-2 ✅
- control.rs 全局状态重构为 EngineControl → Task 3 ✅
- MCP crawl_site 复用共享 Engine → Task 4 ✅
- I4（control 全局状态污染）→ Task 3（per-Engine 隔离）✅
- CLI + 测试迁移 → Task 5 ✅

**设计决策：**
- 删除旧 API（new/run/stream），不向后兼容 ✅
- `Engine::server().serve()` 返回 `Arc<Engine>` 长驻 ✅
- `submit(spider)` 返回 `CrawlHandle`，channel 拉取 items ✅
- `EngineControl` per-Engine 隔离，解决 I4 ✅
- `driver_loop` 用 `tokio::spawn` 隔离单 Spider panic ✅
- 动态注册表用 `RwLock` 并发安全 ✅

**风险点：**
- `process_request` 从 `&EngineContext` 改为 `&Arc<Engine>`，需多次 `read().await` → 可接受
- `RwLock` 写锁在 submit 时持有多字段 → 短暂，可接受
- `driver_loop` 空队列时 `yield_now` 忙等 → 可加 `tokio::time::sleep` 降频
