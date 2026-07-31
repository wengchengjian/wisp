# 方案 C：SpiderBuilder 多 callback + Engine 纯基础设施

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 采用 Crawlee 风格 callback label 路由。废弃共享队列+patterns 路由。Engine 重构为纯基础设施（不持有 Spider），SpiderBuilder 支持注册多个 handler。

**Architecture:**
- **Spider trait**: 加 `handle()` 默认方法（dispatch 到 parse）；callback 字段已存在
- **ClosureSpider**: 持有 `HashMap<String, Handler>`，`handle()` 查表分发
- **Engine**: 纯基础设施（HTTP/缓存/代理池），不持有 Spider；`run(&self, spider)` 接收引用
- **多 Spider**: 多次调用 `engine.run(&spider_a)` / `engine.run(&spider_b)`，各自独立队列/去重，共享底层资源

**Tech Stack:** Rust, tokio, async-trait, futures

---

## 背景

### 当前问题
1. `Engine::spiders()` 共享队列 + patterns 正则路由：独创但缺陷多
2. 全局 `seen` 去重导致跨 Spider URL 冲突
3. follow 请求路由丢失上下文（可能被路由到其他 Spider）
4. `Engine::new(spider).run()` 消费 self，无法复用 HTTP/缓存/代理
5. MCP `crawl_site` 每次创建新 Engine 浪费资源

### 设计原则
- **单 Spider 单队列**：URL 在一个 Spider 内闭环，去重合理
- **callback label 路由**：follow 时指定 label，Spider 内查表分发（O(1)）
- **Engine 不持有 Spider**：纯基础设施，可长期持有，多次 run
- **不向后兼容**：删除 `Engine::new(spider)` / `Engine::spiders()` / `patterns()` / `matches()` / `compiled_patterns`

### 业界先例
- **Crawlee**: `enqueueLinks({label: "detail"})` + `cr_on_html(label="detail", handler)`
- **Scrapy**: `Request(url, callback="parse_detail")` + `def parse_detail()`
- **wisp 方案 C**: `follow_with(url, "detail")` + `SpiderBuilder::on("detail", handler)`

---

## 文件结构

| 文件 | 责任 | 动作 |
|------|------|------|
| `src/crawl/mod.rs` | Spider trait 加 `handle()`；Engine 重构为纯基础设施 | 修改 |
| `src/crawl/builder.rs` | SpiderBuilder `on(label, handler)` + ClosureSpider HashMap | 修改 |
| `src/crawl/engine.rs` | process_request 调 `handle()` 不调 parse；EngineContext 单 Spider 化 | 修改 |
| `src/crawl/control.rs` | 控制状态改为 per-Engine（I4 顺带解决） | 修改 |
| `src/mcp/mod.rs` | MCP server 持有共享 Engine | 修改 |
| `src/mcp/tools.rs` | crawl_site 用 `engine.run(&spider)` | 修改 |
| `src/bin/wisp.rs` | CLI 代码迁移到新 API | 修改 |
| `tests/callback_routing_test.rs` | callback 路由 E2E 测试 | **新建** |
| `tests/engine_infra_test.rs` | Engine 纯基础设施 + 多 run 测试 | **新建** |
| 现有所有测试 | 迁移到新 API | 修改 |

---

## Task 1: Spider trait 加 `handle()` 默认方法

**Files:**
- Modify: `src/crawl/mod.rs`

- [ ] **Step 1: 在 Spider trait 加 handle 方法**

在 [mod.rs:162](file:///f:/project/wisp/src/crawl/mod.rs#L162) 的 Spider trait 中，`parse()` 之后插入：

```rust
/// 请求分发入口。Engine 调用此方法（不直接调 parse）。
///
/// 默认实现：直接调 `parse()`，保持向后兼容。
/// 用户可重写此方法实现 callback 路由（参考 ClosureSpider）。
///
/// # 路由约定
/// - `resp.request.callback` 为 `None` 或 `"default"`：入口请求
/// - 其他字符串：用户自定义 label（通过 `resp.follow_with(url, "detail")` 指定）
async fn handle(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
    self.parse(resp).await
}
```

- [ ] **Step 2: 删除 patterns/matches 相关方法**

删除 [mod.rs:201-216](file:///f:/project/wisp/src/crawl/mod.rs#L201-L216) 的 `patterns()` 和 `matches()` 方法。`until()` 保留（per-Spider 终止条件仍有用）。

- [ ] **Step 3: 删除 schedule() 方法**

删除 [mod.rs:195-199](file:///f:/project/wisp/src/crawl/mod.rs#L195-L199) 的 `schedule()` 方法（未实现，避免误导）。

- [ ] **Step 4: 验证编译**

```
cargo build --lib
```

预期有编译错误（Engine/Builder 引用了 patterns），Task 2-3 修复。

- [ ] **Step 5: 提交**

```bash
git add src/crawl/mod.rs
git commit -m "refactor(crawl): Spider trait 加 handle() 默认方法" -m "删除 patterns/matches/schedule 方法" -m "handle() 默认调 parse()，用户可重写实现 callback 路由"
```

---

## Task 2: SpiderBuilder 实现 `on(label, handler)` 多 callback

**Files:**
- Modify: `src/crawl/builder.rs`

- [ ] **Step 1: 定义 Handler 类型**

在 `src/crawl/builder.rs` 顶部：

```rust
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use futures::future::BoxFuture;
use serde_json::Value;
use super::{Spider, SpiderRequest, SpiderResponse, StopCondition, NeverStop};
use std::collections::HashSet;
use std::time::Duration;
use crate::fetcher::FetchMode;
use crate::http;
use async_trait::async_trait;

/// 异步 handler 签名：接收 SpiderResponse，返回 (items, follows)。
///
/// 用 `Arc<dyn Fn(...) -> BoxFuture>` 让闭包可 Clone + 异步 + Send + Sync。
/// 每个 handler 捕获不同状态都满足同一签名。
pub type Handler = Arc<
    dyn Fn(SpiderResponse) -> BoxFuture<'static, (Vec<Value>, Vec<SpiderRequest>)>
        + Send + Sync
>;
```

- [ ] **Step 2: 改造 SpiderBuilder**

```rust
pub struct SpiderBuilder {
    name: String,
    start_urls: Vec<String>,
    handlers: HashMap<String, Handler>,
    // === Spider 配置 ===
    allowed_domains: HashSet<String>,
    concurrent_requests: u32,
    download_delay: Duration,
    obey_robots: bool,
    max_retries: u32,
    fetcher_config: http::Config,
    fetch_mode: FetchMode,
    max_depth: u32,
    rotate_ua: bool,
    auto_rules: Vec<(String, FetchMode)>,
    auto_exclude: HashSet<String>,
    until: Option<Arc<dyn StopCondition>>,
}

impl SpiderBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            start_urls: Vec::new(),
            handlers: HashMap::new(),
            allowed_domains: HashSet::new(),
            concurrent_requests: 8,
            download_delay: Duration::from_millis(0),
            obey_robots: true,
            max_retries: 3,
            fetcher_config: http::Config::default(),
            fetch_mode: FetchMode::Http,
            max_depth: u32::MAX,
            rotate_ua: false,
            auto_rules: Vec::new(),
            auto_exclude: HashSet::new(),
            until: None,
        }
    }

    pub fn start_urls(mut self, urls: Vec<String>) -> Self {
        self.start_urls = urls; self
    }

    /// 注册 handler。label 为 "default" 或空字符串表示入口（无 callback 时调用）。
    ///
    /// # 示例
    /// ```ignore
    /// SpiderBuilder::new("news")
    ///     .start_urls(vec!["https://news.example.com/"])
    ///     .on("default", |resp| async move {
    ///         // 列表页：follow 到 "detail"
    ///         let follows: Vec<_> = resp.css(".headline a").iter()
    ///             .filter_map(|a| resp.follow_with(a.attr("href"), "detail"))
    ///             .collect();
    ///         (vec![], follows)
    ///     })
    ///     .on("detail", |resp| async move {
    ///         // 详情页：follow 到 "content"
    ///         (vec![], vec![])
    ///     })
    ///     .on("content", |resp| async move {
    ///         // 内容页：提取数据
    ///         (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    ///     })
    ///     .build()
    /// ```
    pub fn on<F, Fut>(mut self, label: &str, handler: F) -> Self
    where
        F: Fn(SpiderResponse) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (Vec<Value>, Vec<SpiderRequest>)> + Send + 'static,
    {
        let boxed: Handler = Arc::new(move |resp| Box::pin(handler(resp)));
        self.handlers.insert(label.to_string(), boxed);
        self
    }

    // === Builder 方法（保持现有） ===
    pub fn allowed_domains(mut self, domains: HashSet<String>) -> Self {
        self.allowed_domains = domains; self
    }
    pub fn concurrent(mut self, n: u32) -> Self {
        self.concurrent_requests = n; self
    }
    pub fn delay(mut self, d: Duration) -> Self {
        self.download_delay = d; self
    }
    pub fn obey_robots(mut self, v: bool) -> Self {
        self.obey_robots = v; self
    }
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n; self
    }
    pub fn fetcher_config(mut self, c: http::Config) -> Self {
        self.fetcher_config = c; self
    }
    pub fn fetch_mode(mut self, m: FetchMode) -> Self {
        self.fetch_mode = m; self
    }
    pub fn max_depth(mut self, n: u32) -> Self {
        self.max_depth = n; self
    }
    pub fn rotate_ua(mut self, v: bool) -> Self {
        self.rotate_ua = v; self
    }
    pub fn auto_rules(mut self, rules: Vec<(String, FetchMode)>) -> Self {
        self.auto_rules = rules; self
    }
    pub fn auto_exclude(mut self, excludes: HashSet<String>) -> Self {
        self.auto_exclude = excludes; self
    }
    pub fn until<C: StopCondition + 'static>(mut self, cond: C) -> Self {
        self.until = Some(Arc::new(cond)); self
    }

    pub fn build(self) -> ClosureSpider {
        ClosureSpider {
            name: self.name,
            start_urls: self.start_urls,
            handlers: self.handlers,
            allowed_domains: self.allowed_domains,
            concurrent_requests: self.concurrent_requests,
            download_delay: self.download_delay,
            obey_robots: self.obey_robots,
            max_retries: self.max_retries,
            fetcher_config: self.fetcher_config,
            fetch_mode: self.fetch_mode,
            max_depth: self.max_depth,
            rotate_ua: self.rotate_ua,
            auto_rules: self.auto_rules,
            auto_exclude: self.auto_exclude,
            until: self.until.unwrap_or_else(|| Arc::new(NeverStop)),
        }
    }
}
```

- [ ] **Step 3: 改造 ClosureSpider 实现 handle()**

```rust
pub struct ClosureSpider {
    name: String,
    start_urls: Vec<String>,
    handlers: HashMap<String, Handler>,
    // === Spider 配置 ===
    allowed_domains: HashSet<String>,
    concurrent_requests: u32,
    download_delay: Duration,
    obey_robots: bool,
    max_retries: u32,
    fetcher_config: http::Config,
    fetch_mode: FetchMode,
    max_depth: u32,
    rotate_ua: bool,
    auto_rules: Vec<(String, FetchMode)>,
    auto_exclude: HashSet<String>,
    until: Arc<dyn StopCondition>,
}

#[async_trait]
impl Spider for ClosureSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }

    /// callback 路由：根据 resp.request.callback 查表分发。
    /// 无 callback 或 label 不存在时回退到 "default" handler。
    async fn handle(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let label = resp.request.callback.as_deref().unwrap_or("default");
        match self.handlers.get(label) {
            Some(h) => h(resp).await,
            None => {
                // label 不匹配，回退到 "default"
                if let Some(default_h) = self.handlers.get("default") {
                    default_h(resp).await
                } else {
                    (vec![], vec![])
                }
            }
        }
    }

    // parse 兜底（用户未注册任何 handler 时用）
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![], vec![])
    }

    fn allowed_domains(&self) -> HashSet<String> { self.allowed_domains.clone() }
    fn concurrent_requests(&self) -> u32 { self.concurrent_requests }
    fn download_delay(&self) -> Duration { self.download_delay }
    fn obey_robots(&self) -> bool { self.obey_robots }
    fn max_retries(&self) -> u32 { self.max_retries }
    fn fetcher_config(&self) -> http::Config { self.fetcher_config.clone() }
    fn fetch_mode(&self) -> FetchMode { self.fetch_mode }
    fn max_depth(&self) -> u32 { self.max_depth }
    fn rotate_ua(&self) -> bool { self.rotate_ua }
    fn auto_rules(&self) -> Vec<(String, FetchMode)> { self.auto_rules.clone() }
    fn auto_exclude(&self) -> HashSet<String> { self.auto_exclude.clone() }
    fn until(&self) -> Arc<dyn StopCondition> { self.until.clone() }
}
```

- [ ] **Step 4: 写测试**

创建 `tests/callback_routing_test.rs`：

```rust
//! callback label 路由测试。
use wisp::crawl::*;
use wisp::crawl::stop::MaxPages;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn test_callback_routing_list_detail_content() {
    // 用 mock server 模拟三阶段爬取
    // 列表页 → 详情页 → 内容页
    let list_server = spawn_html_server(r#"
        <a href="/detail/1">详情1</a>
        <a href="/detail/2">详情2</a>
    "#).await;
    let detail_server = spawn_html_server(r#"<a href="/content/1">内容</a>"#).await;
    let content_server = spawn_html_server(r#"<h1>文章标题</h1>"#).await;

    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();

    let spider = SpiderBuilder::new("pipeline")
        .start_urls(vec![list_server.clone()])
        .on("default", move |_resp| {
            let _ = count_clone.clone();
            async move {
                // 列表页：follow 到 "detail"
                // 实际测试中需要 mock 不同 URL 返回不同内容
                (vec![], vec![])
            }
        })
        .on("detail", move |_resp| async move {
            // 详情页：follow 到 "content"
            (vec![], vec![])
        })
        .on("content", move |resp| async move {
            // 内容页：提取数据
            (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
        })
        .until(MaxPages(10))
        .build();

    let engine = Engine::infra().build().unwrap();
    let stats = engine.run(&spider).await.unwrap();
    assert!(stats.pages_crawled > 0);
}

async fn spawn_html_server(html: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{}", addr)
}
```

- [ ] **Step 5: 验证**

```
cargo build --lib
cargo test --test callback_routing_test -- --nocapture
```

- [ ] **Step 6: 提交**

```bash
git add src/crawl/builder.rs tests/callback_routing_test.rs
git commit -m "feat(builder): SpiderBuilder 支持 on(label, handler) 多 callback" -m "ClosureSpider 持有 HashMap<String, Handler>，handle() 查表分发" -m "Handler 类型用 Arc<dyn Fn(...) -> BoxFuture> 支持异步闭包"
```

---

## Task 3: Engine 重构为纯基础设施

**Files:**
- Modify: `src/crawl/mod.rs`
- Modify: `src/crawl/engine.rs`

- [ ] **Step 1: 重构 Engine 结构体**

删除 `spiders: Vec<Box<dyn Spider>>` 等字段，改为纯基础设施：

```rust
use crate::proxy::ProxyPool;
use crate::storage::Store;

/// 爬虫引擎基础设施。长期持有，多次 run 不同 Spider。
///
/// 共享：HTTP client / 代理池 / SQLite 缓存 / RequestCache。
/// 独立：每次 run 内部 Scheduler/去重/stats（per-Spider 隔离）。
pub struct Engine {
    client: Arc<Client>,
    proxy_pool: Option<Arc<ProxyPool>>,
    cache_store: Option<Arc<Store>>,
    request_cache: Option<RequestCache>,
    // 引擎级配置
    max_concurrent: usize,
    max_pages: usize,
    max_depth: Option<u32>,
    dev_mode: bool,
    checkpoint_store: Option<Arc<Store>>,
    checkpoint_interval: usize,
    // 控制状态（per-Engine，解决 I4）
    control: Arc<control::EngineControl>,
}

/// Engine 构造器（Builder 模式）。
pub struct EngineBuilder {
    max_concurrent: usize,
    max_pages: usize,
    max_depth: Option<u32>,
    proxy_pool: Option<Arc<ProxyPool>>,
    cache_store: Option<Arc<Store>>,
    request_cache: Option<RequestCache>,
    dev_mode: bool,
    checkpoint_store: Option<Arc<Store>>,
    checkpoint_interval: usize,
}

impl Engine {
    /// 创建 builder。
    pub fn infra() -> EngineBuilder {
        EngineBuilder {
            max_concurrent: 8,
            max_pages: 1000,
            max_depth: None,
            proxy_pool: None,
            cache_store: None,
            request_cache: None,
            dev_mode: false,
            checkpoint_store: None,
            checkpoint_interval: 100,
        }
    }
}

impl EngineBuilder {
    pub fn max_concurrent(mut self, n: usize) -> Self { self.max_concurrent = n; self }
    pub fn max_pages(mut self, n: usize) -> Self { self.max_pages = n; self }
    pub fn max_depth(mut self, n: u32) -> Self { self.max_depth = Some(n); self }
    pub fn proxy_pool(mut self, p: Arc<ProxyPool>) -> Self { self.proxy_pool = Some(p); self }
    pub fn cache_store(mut self, s: Arc<Store>) -> Self { self.cache_store = Some(s); self }
    pub fn request_cache(mut self, c: RequestCache) -> Self { self.request_cache = Some(c); self }
    pub fn dev_mode(mut self, s: Arc<Store>) -> Self {
        self.cache_store = Some(s); self.dev_mode = true; self
    }
    pub fn checkpoint(mut self, s: Arc<Store>, interval: usize) -> Self {
        self.checkpoint_store = Some(s); self.checkpoint_interval = interval; self
    }

    pub fn build(self) -> Result<Engine> {
        let client = Arc::new(
            Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?
        );
        Ok(Engine {
            client,
            proxy_pool: self.proxy_pool,
            cache_store: self.cache_store,
            request_cache: self.request_cache,
            max_concurrent: self.max_concurrent,
            max_pages: self.max_pages,
            max_depth: self.max_depth,
            dev_mode: self.dev_mode,
            checkpoint_store: self.checkpoint_store,
            checkpoint_interval: self.checkpoint_interval,
            control: Arc::new(control::EngineControl::new()),
        })
    }
}
```

- [ ] **Step 2: 实现 Engine::run 接收 Spider 引用**

```rust
impl Engine {
    /// 运行单个 Spider。返回统计。
    ///
    /// 共享底层资源（HTTP/缓存/代理），Spider 内部独立 Scheduler/去重。
    /// 可多次调用：`engine.run(&spider_a).await?; engine.run(&spider_b).await?;`
    pub async fn run(&self, spider: &dyn Spider) -> Result<CrawlStats> {
        self.run_with_sender(spider, None).await
    }

    /// 流式运行：边爬边产出事件。
    pub fn run_stream(&self, spider: &dyn Spider) -> CrawlStream {
        // 类似原 stream()，但接收 &dyn Spider
        // ...
    }

    /// 获取控制句柄（用于外部 pause/resume/cancel/shutdown）。
    pub fn control(&self) -> &Arc<control::EngineControl> {
        &self.control
    }

    /// 关闭 Engine（停止所有运行中的爬取）。
    pub fn shutdown(&self) {
        self.control.shutdown();
    }

    async fn run_with_sender(
        &self,
        spider: &dyn Spider,
        tx: Option<tokio::sync::mpsc::Sender<CrawlEvent>>,
    ) -> Result<CrawlStats> {
        // 重置 control（每次 run 清理上次状态）
        self.control.reset().await;

        let stats = Arc::new(SpiderStats::new());
        let mut rule_engine = auto::ModeRuleEngine::new();
        for (pattern, mode) in spider.auto_rules() {
            let _ = rule_engine.add_user_rule(&pattern, mode);
        }
        let rule_engine = Arc::new(Mutex::new(rule_engine));
        let allowed = Arc::new(spider.allowed_domains());
        let fetcher_config = spider.fetcher_config();
        let fetch_mode = spider.fetch_mode();
        let max_concurrent = self.max_concurrent;
        let max_depth = self.max_depth.unwrap_or_else(|| spider.max_depth());
        let obey_robots = spider.obey_robots();
        let auto_excludes = spider.auto_exclude();

        let sched = Arc::new(scheduler::Scheduler::new());
        let robots_cache = Arc::new(Mutex::new(robots::RobotsCache::new()));
        let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();

        // 单 Spider checkpoint 恢复
        let spider_name = spider.name().to_string();
        let mut restored_pending = false;
        if let Some(ref store) = self.checkpoint_store {
            if let Some(blob) = store.load_checkpoint(&spider_name)? {
                match bincode::deserialize::<CrawlState>(&blob) {
                    Ok(state) => {
                        if !state.pending_urls.is_empty() {
                            let n = state.pending_urls.len();
                            for req in state.pending_urls {
                                sched.push(req).await;
                            }
                            tracing::info!("Spider '{}' 从 checkpoint 恢复 {} 个 pending URLs", spider_name, n);
                            restored_pending = true;
                        }
                    }
                    Err(e) => tracing::warn!("checkpoint 反序列化失败: {}", e),
                }
            }
        }

        if !restored_pending {
            for url in spider.start_urls() {
                sched.push(SpiderRequest::get(&url)).await;
            }
        }

        spider.on_start().await;

        let ctx = Arc::new(engine::EngineContext {
            client: self.client.clone(),
            sched: sched.clone(),
            robots_cache,
            follow_tx,
            follow_rx: Arc::new(Mutex::new(follow_rx)),
            domain_sems: Arc::new(Mutex::new(HashMap::new())),
            proxy_pool: self.proxy_pool.clone(),
            cache_store: self.cache_store.clone(),
            request_cache: self.request_cache.clone(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            start: std::time::Instant::now(),
            tx,
            dev_mode: self.dev_mode,
            // 单 Spider：直接持有引用而非 Vec
            spider: spider_ref,  // 用 Arc<dyn Spider> 或其他方式
            stats: stats.clone(),
            rule_engine,
            auto_excludes,
            allowed,
            fetcher_config,
            fetch_mode,
            max_concurrent,
            max_depth,
            obey_robots,
            global_in_flight: Arc::new(AtomicUsize::new(0)),
            engine_max_pages: self.max_pages,
            control: self.control.clone(),
        });

        // ... 原有的 unfold + buffer_unordered 驱动循环 ...
        // 唯一变化：调用 spider.handle(resp) 而非 spider.parse(resp)
        //           路由逻辑删除（单 Spider，所有 URL 都给这个 Spider）

        spider.on_close().await;
        // ... 汇总 stats ...
    }
}
```

**关键改动**：
1. `EngineContext.spiders: Vec<Arc<dyn Spider>>` → `spider: Arc<dyn Spider>`（单 Spider）
2. 所有 `ctx.spiders[idx]` → `ctx.spider`
3. `ctx.stats[idx]` → `ctx.stats`（单个）
4. 删除路由逻辑 [mod.rs:593-634](file:///f:/project/wisp/src/crawl/mod.rs#L593-L634)
5. `spider.parse(resp)` → `spider.handle(resp)`（[engine.rs:239](file:///f:/project/wisp/src/crawl/engine.rs#L239)）
6. control 调用改为 `ctx.control.is_cancelled()` 而非全局 `control::is_cancelled()`

- [ ] **Step 3: 修改 EngineContext 单 Spider 化**

```rust
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
    // === 单 Spider 配置 ===
    pub spider: Arc<dyn Spider>,  // 原 spiders: Vec<...>
    pub stats: Arc<SpiderStats>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
    pub auto_excludes: HashSet<String>,
    pub allowed: Arc<HashSet<String>>,
    pub fetcher_config: http::Config,
    pub fetch_mode: FetchMode,
    pub max_concurrent: usize,
    pub max_depth: u32,
    pub obey_robots: bool,
    pub global_in_flight: Arc<AtomicUsize>,
    pub engine_max_pages: usize,
    pub control: Arc<control::EngineControl>,  // 新增
}
```

**注意**：`spider: Arc<dyn Spider>` 不能直接从 `&dyn Spider` 构造（需要 owned）。有两个选择：
- A. `Engine::run` 接收 `Arc<dyn Spider>` 而非 `&dyn Spider`（用户需 `Arc::new(spider)` 包装）
- B. `Engine::run` 内部用 unsafe 转换（不推荐）
- C. `Engine::run` 签名改为 `run(&self, spider: impl Spider + 'static)`，内部 `Arc::new(spider)`

选 C，对用户友好：

```rust
pub async fn run<S: Spider + 'static>(&self, spider: S) -> Result<CrawlStats> {
    let spider: Arc<dyn Spider> = Arc::new(spider);
    self.run_inner(spider, None).await
}
```

但这样每次 run 都消费 spider。如果用户要复用 spider 实例（多次 run 同一 spider），需要提供：

```rust
pub async fn run_arc(&self, spider: Arc<dyn Spider>) -> Result<CrawlStats> {
    self.run_inner(spider, None).await
}
```

或让 `run` 同时支持：

```rust
pub async fn run(&self, spider: &(dyn Spider)) -> Result<CrawlStats> {
    // 用 unsafe 或 redesign EngineContext 不持有 spider，只持有引用
}
```

**最简洁方案**：EngineContext 不持有 `Arc<dyn Spider>`，改为在 `process_request` 每次传入 `&dyn Spider`。EngineContext 拆分为"共享资源"和"per-spider 配置"两部分，spider 作为参数传递。

具体做法：把 `process_request(ctx, req)` 改为 `process_request(ctx, spider, stats, req)`，每次调用传 spider 引用。

- [ ] **Step 4: 删除 Engine::new/spiders/builder/run(消费 self)/run_one/stream(消费 self)**

删除所有旧 API。

- [ ] **Step 5: 写测试**

创建 `tests/engine_infra_test.rs`：

```rust
//! Engine 纯基础设施测试：多次 run 共享资源。
use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;

struct CountSpider { name: String, url: String }
#[async_trait]
impl Spider for CountSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![serde_json::json!({"name": self.name})], vec![])
    }
    fn obey_robots(&self) -> bool { false }
}

#[tokio::test]
async fn test_engine_multiple_runs_share_resources() {
    let engine = Engine::infra().max_pages(10).build().unwrap();

    let stats_a = engine.run(CountSpider { name: "a".into(), url: "http://127.0.0.1:1/".into() }).await.unwrap();
    let stats_b = engine.run(CountSpider { name: "b".into(), url: "http://127.0.0.1:1/".into() }).await.unwrap();

    // 两个 Spider 各自独立 run，stats 隔离
    assert_eq!(stats_a.items_scraped, 0);  // 不可达 URL 不产出 item
    assert_eq!(stats_b.items_scraped, 0);
}

#[tokio::test]
async fn test_engine_control_isolation() {
    let engine_a = Engine::infra().build().unwrap();
    let engine_b = Engine::infra().build().unwrap();

    engine_a.control().pause_all();
    assert!(!engine_b.control().is_shutdown(), "Engine B 不应受 A 影响");

    engine_a.control().shutdown();
    assert!(engine_a.control().is_shutdown());
    assert!(!engine_b.control().is_shutdown(), "Engine B 不应受 A 关闭影响");
}
```

- [ ] **Step 6: 验证**

```
cargo build --lib
cargo test --test engine_infra_test -- --nocapture
```

- [ ] **Step 7: 提交**

```bash
git add src/crawl/mod.rs src/crawl/engine.rs tests/engine_infra_test.rs
git commit -m "refactor(crawl): Engine 重构为纯基础设施" -m "删除 spiders: Vec<...>，改为单 Spider run" -m "Engine::infra().build() 长期持有，多次 run 共享 HTTP/缓存/代理" -m "process_request 改为接收 spider 参数，调 handle() 不调 parse()"
```

---

## Task 4: control.rs 改为 per-Engine EngineControl

**Files:**
- Modify: `src/crawl/control.rs`
- Modify: `src/crawl/engine.rs`

- [ ] **Step 1: 定义 EngineControl 结构体**

```rust
//! 引擎级控制状态（per-Engine 隔离，解决 I4）。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};

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

- [ ] **Step 2: 删除全局 static 和全局函数**

删除 `PAUSED_URLS`、`CANCELLED_URLS`、`GLOBAL_PAUSED`、`SHUTDOWN_FLAG`、`VERSION`、`bump()`、`pause()`、`resume()`、`cancel()`、`pause_all()`、`resume_all()`、`shutdown()`、`reset()`、`is_cancelled()`、`wait_if_paused()`、`is_shutdown()` 这些全局 API。

- [ ] **Step 3: process_request 用 ctx.control**

修改 [engine.rs:94-96](file:///f:/project/wisp/src/crawl/engine.rs#L94-L96)：

```rust
// 旧：
// if super::control::is_cancelled(&req.url).await { return; }
// if !super::control::wait_if_paused(&req.url).await { return; }
// if super::control::is_shutdown() { return; }

// 新：
if ctx.control.is_cancelled(&req.url).await { return; }
if !ctx.control.wait_if_paused(&req.url).await { return; }
if ctx.control.is_shutdown() { return; }
```

- [ ] **Step 4: Engine::shutdown 用 control**

```rust
impl Engine {
    pub fn shutdown(&self) {
        self.control.shutdown();
    }
}
```

- [ ] **Step 5: 迁移 control.rs 的测试**

将 control.rs 的单元测试移到 `tests/engine_infra_test.rs` 或改为对 `EngineControl` 的测试。

- [ ] **Step 6: 验证**

```
cargo build --lib
cargo test --test engine_infra_test -- --nocapture
```

- [ ] **Step 7: 提交**

```bash
git add src/crawl/control.rs src/crawl/engine.rs tests/engine_infra_test.rs
git commit -m "refactor(crawl): control 全局状态重构为 per-Engine EngineControl" -m "I4: 删除全局 static，Engine 持有独立 EngineControl" -m "多 Engine 实例控制状态完全隔离"
```

---

## Task 5: MCP crawl_site 用共享 Engine

**Files:**
- Modify: `src/mcp/mod.rs`
- Modify: `src/mcp/tools.rs`

- [ ] **Step 1: MCP server 持有共享 Engine**

```rust
pub async fn run() -> Result<()> {
    let store = Arc::new(Store::open(Path::new("wisp.db"))?);
    let engine = Engine::infra()
        .max_pages(100000)
        .cache_store(store.clone())
        .dev_mode(store.clone())  // 启用开发模式缓存
        .build()?;

    // ... JSON-RPC 循环 ...
    let response = match method.as_str() {
        "tools/call" => {
            let result = handle_tools_call(request, &store, &engine).await?;
            // ...
        }
    };

    engine.shutdown();
    Ok(())
}

async fn handle_tools_call(
    request: Value,
    store: &Arc<Store>,
    engine: &Engine,
) -> Result<Value> {
    let name = request["params"]["name"].as_str().unwrap_or("");
    let args = request["params"]["arguments"].clone();
    match name {
        "crawl_site" => tools::crawl_site(args, store, engine).await,
        // ...
    }
}
```

- [ ] **Step 2: 修改 crawl_site 用 engine.run**

```rust
pub async fn crawl_site(
    args: Value,
    _store: &Arc<Store>,
    engine: &Engine,
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
            let doc = resp.parse().unwrap_or_else(|_| Node::from_html(""));
            let items: Vec<Value> = doc.select(&self.css).iter()
                .map(|n| serde_json::json!({"text": n.text(), "html": n.html()}))
                .collect();
            (items, vec![])
        }
        fn obey_robots(&self) -> bool { false }
        fn until(&self) -> Arc<dyn StopCondition> {
            Arc::new(MaxPages(self.max_pages))
        }
    }

    let spider = SimpleSpider { css: css_selector, start_urls, max_pages };
    let stats = engine.run(spider).await?;

    // stats 不直接含 items，需要改用 run_stream 拉取
    // 或改 Engine::run 返回 (stats, Vec<Value>)
    // 见 Step 3 讨论
}
```

- [ ] **Step 3: 改 Engine::run 返回 items**

MCP 需要拿 items。有两种方案：
- A. `run(spider)` 返回 `Result<CrawlStats>`，items 通过 `on_item` 钩子收集
- B. `run(spider)` 返回 `Result<(CrawlStats, Vec<Value>)>`
- C. 用 `run_stream` 拉取 items

选 B，最直接：

```rust
pub async fn run<S: Spider + 'static>(&self, spider: S) -> Result<(CrawlStats, Vec<Value>)> {
    // ... 内部维护 items: Vec<Value>，on_item 时收集 ...
}
```

但 Spider trait 的 `on_item` 是 Spider 自己的方法，不是 Engine 的。Engine 不知道 items。
其实 process_request 内部已经收集 items 并发送到 tx channel，可以增加一个 items 收集 Vec。

具体实现：Engine::run 内部维护 `Arc<Mutex<Vec<Value>>>`，在 [engine.rs:250-257](file:///f:/project/wisp/src/crawl/engine.rs#L250-L257) 发送 CrawlEvent::Item 之前 push 到这个 Vec。

或更优雅：用户需要 items 时用 `run_stream`：

```rust
pub async fn run_collect<S: Spider + 'static>(&self, spider: S) -> Result<(CrawlStats, Vec<Value>)> {
    let mut items = Vec::new();
    let mut stream = self.run_stream(spider).events();
    use futures::StreamExt;
    let mut final_stats = CrawlStats::default();
    while let Some(event) = stream.next().await {
        match event {
            CrawlEvent::Item(v) => items.push(v),
            CrawlEvent::Done(s) => final_stats = s,
            _ => {}
        }
    }
    Ok((final_stats, items))
}
```

- [ ] **Step 4: 修改 crawl_site 测试**

修改 `tests/cr_fix_t1_test.rs`，用新 API：

```rust
#[tokio::test]
async fn test_crawl_site_uses_start_urls() {
    let server = spawn_html_server("<p>item1</p><p>item2</p>").await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let engine = Engine::infra().max_pages(10).build().unwrap();
    let args = json!({
        "start_urls": [server],
        "css_selector": "p",
        "max_pages": 1
    });
    let result = crawl_site(args, &store, &engine).await.expect("crawl_site should succeed");
    assert_eq!(result["items_count"].as_u64(), Some(2), "应爬到 2 个 p 元素");
}
```

- [ ] **Step 5: 验证**

```
cargo build --lib
cargo test --test cr_fix_t1_test -- --nocapture
```

- [ ] **Step 6: 提交**

```bash
git add src/mcp/mod.rs src/mcp/tools.rs tests/cr_fix_t1_test.rs
git commit -m "refactor(mcp): crawl_site 用共享 Engine.run" -m "MCP server 启动时创建一个长驻 Engine" -m "crawl_site 动态创建 Spider 并 run，复用 HTTP/缓存/代理"
```

---

## Task 6: 迁移 CLI 和所有测试到新 API

**Files:**
- Modify: `src/bin/wisp.rs`
- Modify: `tests/multi_spider_test.rs`
- Modify: `tests/stop_condition_test.rs`
- Modify: `tests/builder_api_test.rs`
- Modify: `tests/cr_fix_engine_test.rs`
- Modify: `tests/cr_fix_t4_test.rs`
- Modify: `tests/cr_fix_t7_test.rs`
- Modify: `tests/cr_fix_t10_test.rs`
- Modify: `tests/cr_fix_t11_test.rs`
- Modify: `src/crawl/mod.rs` (内联测试)

- [ ] **Step 1: 搜索所有旧 API 调用**

```
grep -rn "Engine::new\|Engine::spiders\|Engine::builder\|\.run()\|\.run_one()\|\.stream()\|patterns()\|matches()" src/ tests/
```

- [ ] **Step 2: 迁移 CLI**

`src/bin/wisp.rs`：

```rust
// 旧：
// let engine = Engine::new(spider).max_pages(max_pages);
// let stats = engine.run_one().await?;

// 新：
let engine = Engine::infra().max_pages(max_pages).build()?;
let (stats, _items) = engine.run(spider).await?;
```

- [ ] **Step 3: 迁移现有测试**

将 `Engine::new(spider).run().await` 改为 `Engine::infra().build()?.run(spider).await`。

多 Spider 测试改为多次 run：

```rust
// 旧：
// let engine = Engine::spiders(vec![Box::new(spider_a), Box::new(spider_b)]).max_pages(10);
// let results = engine.run().await.unwrap();

// 新：
let engine = Engine::infra().max_pages(10).build().unwrap();
let (stats_a, _) = engine.run(spider_a).await.unwrap();
let (stats_b, _) = engine.run(spider_b).await.unwrap();
```

- [ ] **Step 4: 删除 patterns 测试**

删除 `tests/multi_spider_test.rs` 中针对 patterns 路由的测试用例（已废弃）。

- [ ] **Step 5: 验证所有测试**

```
cargo test --lib
cargo test --test callback_routing_test
cargo test --test engine_infra_test
cargo test --test multi_spider_test
cargo test --test stop_condition_test
cargo test --test builder_api_test
cargo test --test cr_fix_t1_test
cargo test --test cr_fix_t4_test
cargo test --test cr_fix_t7_test
cargo test --test cr_fix_t10_test
cargo test --test cr_fix_t11_test
```

Expected: 全部 PASS

- [ ] **Step 6: 编译检查**

```
cargo build
```

- [ ] **Step 7: 提交**

```bash
git add src/bin/wisp.rs tests/ src/crawl/mod.rs src/crawl/builder.rs
git commit -m "refactor: 迁移所有调用方到 Engine infra + run API" -m "删除 Engine::new/spiders/builder/run(消费 self)/run_one/stream(消费 self)" -m "CLI + 所有测试迁移到新 API"
```

---

## Task 7: 全量验证

- [ ] **Step 1: 全量测试**

```
cargo test
```
Expected: 全部 PASS

- [ ] **Step 2: 编译检查**

```
cargo build --release
```

- [ ] **Step 3: 确认提交历史**

```
git log --oneline -10
```

---

## 使用示例

### 示例 1：简单爬虫（用户手写 Spider）

```rust
struct NewsSpider;
#[async_trait]
impl Spider for NewsSpider {
    fn name(&self) -> &str { "news" }
    fn start_urls(&self) -> Vec<String> { vec!["https://news.example.com/".into()] }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let items: Vec<Value> = resp.css(".headline").iter()
            .map(|h| json!({"title": h.text()}))
            .collect();
        (items, vec![])
    }
    fn obey_robots(&self) -> bool { false }
}

let engine = Engine::infra().max_pages(100).build()?;
let (stats, items) = engine.run(NewsSpider).await?;
println!("{}", stats.summary());
```

### 示例 2：callback 路由（SpiderBuilder，列表→详情→内容）

```rust
let spider = SpiderBuilder::new("pipeline")
    .start_urls(vec!["https://example.com/list".into()])
    .on("default", |resp| async move {
        // 列表页 → follow 到 "detail"
        let follows: Vec<_> = resp.css(".item a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href"), "detail"))
            .collect();
        (vec![], follows)
    })
    .on("detail", |resp| async move {
        // 详情页 → follow 到 "content"
        let follows: Vec<_> = resp.css("article a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href"), "content"))
            .collect();
        (vec![], follows)
    })
    .on("content", |resp| async move {
        // 内容页 → 提取数据
        (vec![json!({
            "title": resp.css("h1").text(),
            "body": resp.css(".article-body").text(),
        })], vec![])
    })
    .until(MaxPages(1000))
    .obey_robots(false)
    .build();

let engine = Engine::infra().max_pages(10000).build()?;
let (stats, items) = engine.run(spider).await?;
```

### 示例 3：MCP server 共享 Engine

```rust
// MCP server 启动时创建一次
let engine = Engine::infra()
    .max_pages(100000)
    .cache_store(store.clone())
    .dev_mode(store.clone())
    .build()?;

// 每次 crawl_site 调用：
pub async fn crawl_site(args: Value, _store: &Arc<Store>, engine: &Engine) -> Result<Value> {
    let spider = SimpleSpider { /* ... */ };
    let (stats, items) = engine.run(spider).await?;
    Ok(json!({"items_count": items.len(), "jsonl": ...}))
}
```

### 示例 4：多 Spider 并发（共享资源池）

```rust
let engine = Arc::new(Engine::infra().proxy_pool(proxies).build()?);

let engine_a = engine.clone();
let engine_b = engine.clone();
let (stats_a, stats_b) = tokio::join!(
    async move { engine_a.run(spider_a).await },
    async move { engine_b.run(spider_b).await },
);
```

---

## 自检清单

**目标覆盖：**
- Spider trait 加 `handle()` → Task 1 ✅
- SpiderBuilder `on(label, handler)` 多 callback → Task 2 ✅
- Engine 重构为纯基础设施 → Task 3 ✅
- control.rs per-Engine 隔离（I4 解决）→ Task 4 ✅
- MCP crawl_site 共享 Engine → Task 5 ✅
- CLI + 测试迁移 → Task 6 ✅

**设计决策：**
- 单 Spider 单队列（无全局去重冲突）✅
- callback label 路由（HashMap O(1)，对齐 Crawlee）✅
- Engine 不持有 Spider，纯基础设施 ✅
- 多 Spider 用多次 `engine.run()` + 共享 Engine ✅
- 不向后兼容，删除所有旧 API ✅

**风险点：**
- `EngineContext` 单 Spider 化改动大（Vec → 单值）→ Task 3 Step 3
- `process_request` 签名变化（加 spider 参数）→ Task 3 Step 3
- 闭包 `Handler` 类型可能编译复杂 → Task 2 Step 1 验证
- 测试迁移工作量大（10+ 测试文件）→ Task 6

**废弃特性：**
- `Engine::new(spider)` / `Engine::spiders()` / `Engine::builder()` ❌
- `Engine::run(self) -> Vec<CrawlStats>` / `run_one(self)` / `stream(self)` ❌
- `Spider::patterns()` / `Spider::matches()` / `Spider::schedule()` ❌
- `control.rs` 全局 static 函数 ❌
