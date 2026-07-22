# wisp

Rust 爬虫框架。提供 Spider trait（含 callback 路由的 `handle` 方法）、SpiderBuilder 多 handler 构建、Engine 纯基础设施（HTTP/缓存/代理池共享）。

## 核心概念

### Spider trait (`src/crawl/mod.rs`)

用户实现的核心 trait，定义爬虫行为：

```rust
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    // 必需
    fn name(&self) -> &str;
    fn start_urls(&self) -> Vec<String>;
    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>);

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

    // 可选钩子（带默认值）
    fn allowed_domains(&self) -> HashSet<String> { HashSet::new() }
    fn download_delay(&self) -> Duration { Duration::from_millis(0) }
    fn obey_robots(&self) -> bool { true }
    fn max_retries(&self) -> u32 { 3 }
    fn fetcher_config(&self) -> http::Config { http::Config::default() }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
    async fn on_error(&self, _req: &SpiderRequest, _err: &str) {}
    async fn on_item(&self, item: Value) -> Option<Value> { Some(item) }
    fn is_blocked(&self, resp: &SpiderResponse) -> bool { /* 401/403/... */ }
    fn fetch_mode(&self) -> FetchMode { FetchMode::Http }
    fn auto_rules(&self) -> Vec<(String, FetchMode)> { Vec::new() }
    fn auto_exclude(&self) -> HashSet<String> { HashSet::new() }
    fn max_depth(&self) -> u32 { u32::MAX }
    async fn on_before_request(&self, _req: &SpiderRequest) -> RequestAction { RequestAction::Proceed }

    // === 终止策略 ===
    fn until(&self) -> Arc<dyn StopCondition> { Arc::new(NeverStop) }
}
```

### Callback 路由（核心机制）

Engine 调用 `Spider::handle(resp)` 而非 `Spider::parse(resp)`，实现 per-request label 路由：

1. 起始 URL 入队时 `callback = None`，由 `handle()` 路由到 `"default"` handler
2. handler 内通过 `resp.follow_with(href, "detail")` 生成带 label 的新请求
3. 该请求被 Engine 取回后，`resp.request.callback = Some("detail")`，再次由 `handle()` 分发到对应 handler

默认 `handle()` 实现直接调 `parse()`；用户通过 `SpiderBuilder::on(label, handler)` 注册的多 handler 由 `ClosureSpider::handle` 统一查表分发。

### 终止策略

#### `Spider::until(&self) -> Arc<dyn StopCondition>`

per-spider 终止策略。默认 `NeverStop`（由引擎 `max_pages` 兜底）。
返回 `Arc<dyn StopCondition>`，可组合使用 `and` / `or` / `not`。

### SpiderBuilder (`src/crawl/builder.rs`)

闭包式 Spider 构建，避免手写 trait impl。通过 `on(label, handler)` 注册多 callback handler：

```rust
use wisp::crawl::SpiderBuilder;
use wisp::crawl::stop::{MaxPages, Timeout};
use std::time::Duration;

// 简单爬虫（单 default handler）
let spider = SpiderBuilder::new("quotes")
    .start_urls(vec!["https://quotes.toscrape.com/"])
    .delay(Duration::from_millis(500))
    .obey_robots(false)
    .on("default", |resp| async move {
        let doc = resp.parse().unwrap();
        let items = doc.select(".quote").iter().map(|q| {
            serde_json::json!({ "text": q.select_one(".text").map(|n| n.text()) })
        }).collect();
        (items, vec![])
    })
    .until(MaxPages(100).or(Timeout(Duration::from_secs(60))))
    .build();
```

多 callback 路由（列表 → 详情 → 内容）：

```rust
let spider = SpiderBuilder::new("pipeline")
    .start_urls(vec!["https://example.com/list"])
    .on("default", |resp| async move {
        // 列表页：follow 到 "detail"
        let follows: Vec<_> = resp.css(".item a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "detail"))
            .collect();
        (vec![], follows)
    })
    .on("detail", |resp| async move {
        // 详情页：follow 到 "content"
        let follows: Vec<_> = resp.css("article a").iter()
            .filter_map(|a| resp.follow_with(a.attr("href").unwrap_or(""), "content"))
            .collect();
        (vec![], follows)
    })
    .on("content", |resp| async move {
        // 内容页：提取数据
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .until(MaxPages(1000))
    .build();
```

#### `SpiderBuilder::on<F, Fut>(label, handler)`

注册 handler。`label` 为 `"default"` 表示入口（无 callback 时调用）。
多 callback 路由：`resp.follow_with(url, "detail")` 产生的请求由 `on("detail", handler)` 处理。
至少注册一个 handler 才能 `build()`。

#### `SpiderBuilder::sitemap(name, sitemap_urls, content_label)`

预设：自动解析 sitemap.xml，提取 `<loc>` URL，follow 到指定 label 的 handler：

```rust
let spider = SpiderBuilder::sitemap("my_spider", vec!["https://x.com/sitemap.xml".into()], "content")
    .on("content", |resp| async move {
        (vec![serde_json::json!({"title": resp.css("h1").text()})], vec![])
    })
    .build();
```

#### `SpiderBuilder::until<C: StopCondition>(cond: C)`

设置终止条件。等价于在 Spider impl 中重写 `until()`。

### StopCondition (`src/crawl/stop.rs`)

终止策略 trait，返回 `true` 表示该 Spider 停止派发新请求：

```rust
pub trait StopCondition: Send + Sync {
    fn should_stop(&self, ctx: &StopContext) -> bool;
    fn and<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>;
    fn or<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>;
    fn not(self) -> Arc<dyn StopCondition>;
}
```

内置原子策略：
- `MaxPages(usize)` — 已爬页数达到上限
- `MaxItems(usize)` — 已产 item 数达到上限
- `MaxErrors(usize)` — 错误数达到上限
- `Timeout(Duration)` — 运行时长达到上限
- `NeverStop` — 永不停止（默认）
- `FnStopCondition(F)` — 闭包转 StopCondition

### Engine (`src/crawl/mod.rs`)

纯基础设施：长期持有 HTTP client / 代理池 / SQLite 缓存 / RequestCache，不持有 Spider。
通过 `Engine::infra()` 构造 builder，`.build()` 生成 `Engine`，可多次调用 `run(spider)`：

```rust
use wisp::crawl::Engine;

// 构造 Engine（一次性，长期持有）
let engine = Engine::infra()
    .max_concurrent(8)
    .max_pages(10000)
    .build()?;

// 多次运行不同 Spider（共享底层资源，Spider 之间独立 Scheduler/去重/stats）
let (stats_a, items_a) = engine.run(spider_a).await?;
let (stats_b, items_b) = engine.run(spider_b).await?;
```

#### `Engine::infra() -> EngineBuilder`

创建 Engine builder（替代原 `Engine::new(spider)` / `Engine::spiders(vec)` / `Engine::builder(spider)`）。

#### `Engine::run<S: Spider + 'static>(&self, spider: S) -> Result<(CrawlStats, Vec<Value>)>`

运行单个 Spider，返回 (统计, items)。每次调用重置 `EngineControl`，清理上次的 pause/cancel/shutdown 状态。

#### `Engine::run_stream<S: Spider + 'static>(&self, spider: S) -> CrawlStream`

流式运行：边爬边产出 `CrawlEvent`（`Item` / `PageScraped` / `Error` / `Done`）。

#### `EngineBuilder` 链式配置

- `.max_concurrent(usize)` — 并发数（默认 8）
- `.max_pages(usize)` — 全局页数硬上限（默认 1000）
- `.max_depth(u32)` — 最大深度
- `.proxy_pool(Arc<ProxyPool>)` — 共享代理池
- `.cache_store(Arc<Store>)` — 共享 SQLite 缓存
- `.request_cache(RequestCache)` — 请求级缓存
- `.dev_mode(Arc<Store>)` — 开发模式（自动落缓存快照）
- `.checkpoint(Arc<Store>, interval)` — checkpoint 存储 + 间隔
- `.build() -> Result<Engine>`

### SpiderResponse (`src/crawl/mod.rs`)

爬取响应。`from_cache` 字段标记是否来自缓存（命中缓存不计入 `pages_crawled`）：

```rust
pub struct SpiderResponse {
    pub url: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub request: SpiderRequest,
    #[doc(hidden)] pub tracker: Option<Arc<Mutex<SelectorTracker>>>,
    #[doc(hidden)] pub from_cache: bool,  // 是否命中请求缓存
}
```

`from_cache` 和 `tracker` 虽标记为 `#[doc(hidden)]`，但属于 pub 字段，测试中构造时需显式写出：
`tracker: None, from_cache: false`。

便捷方法：
- `resp.text()` / `resp.parse()` / `resp.json()` — 解码 body
- `resp.follow(href)` — 相对链接转 GET 请求（depth 自动 +1）
- `resp.follow_with(href, "detail")` — 同上，但带 callback label
- `resp.follow_meta(href, meta)` — 同上，但带 meta
- `resp.css(sel)` — CSS 查询（Auto 模式自动追踪选择器匹配数）
- `resp.xpath_auto(expr)` — XPath 查询（Auto 模式自动追踪）

## 测试

```bash
cargo test --lib                    # lib 单元测试
cargo test --test stop_condition_test
cargo test --test builder_api_test
cargo test --test multi_spider_test

# 真实网络测试（需网络与代理 127.0.0.1:7897）
cargo test --test real_scrape_test -- --ignored
cargo test --test cf_bypass_real_test -- --ignored
```

注意：`tests/real_scrape_test.rs`、`tests/cf_bypass_real_test.rs` 存在预先的 GBK 编码问题（非 UTF-8），需单独修复编码后才能编译。

## 构建

```bash
cargo build            # lib + bins
cargo build --release
```
