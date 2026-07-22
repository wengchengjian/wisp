# wisp

Rust 爬虫框架。提供 Spider trait、SpiderBuilder 闭包式构建、多 Spider 共享队列引擎。

## 核心概念

### Spider trait (`src/crawl/mod.rs`)

用户实现的核心 trait，定义爬虫行为：

```rust
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn start_urls(&self) -> Vec<String>;
    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>);

    // 可选钩子（带默认值）
    fn allowed_domains(&self) -> HashSet<String> { HashSet::new() }
    fn concurrent_requests(&self) -> u32 { 8 }
    fn download_delay(&self) -> Duration { Duration::from_millis(0) }
    fn obey_robots(&self) -> bool { true }
    fn max_retries(&self) -> u32 { 3 }
    fn fetcher_config(&self) -> http::Config { http::Config::default() }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
    async fn on_error(&self, _req: &SpiderRequest, _err: &str) {}
    async fn on_item(&self, item: Value) -> Option<Value> { Some(item) }
    fn is_blocked(&self, resp: &SpiderResponse) -> bool { /* 401/403/... */ }
    fn configure_sessions(&self, _mgr: &mut session::SessionManager) {}
    fn session_for(&self, _req: &SpiderRequest) -> &str { "default" }
    fn fetch_mode(&self) -> FetchMode { FetchMode::Http }
    fn auto_rules(&self) -> Vec<(String, FetchMode)> { Vec::new() }
    fn auto_exclude(&self) -> HashSet<String> { HashSet::new() }
    fn max_depth(&self) -> u32 { u32::MAX }
    fn rotate_ua(&self) -> bool { false }
    async fn on_before_request(&self, _req: &SpiderRequest) -> RequestAction { RequestAction::Proceed }
    fn schedule(&self) -> Option<&str> { None } // Cron 表达式

    // === 路由与终止（Task 1-9 新增） ===
    fn patterns(&self) -> Vec<String> { Vec::new() }
    fn matches(&self, url: &str) -> bool { /* 默认实现 */ }
    fn until(&self) -> Arc<dyn StopCondition> { Arc::new(NeverStop) }
}
```

### 路由与终止策略（Task 1-9 重构新增）

#### `Spider::patterns(&self) -> Vec<String>`

URL 路由匹配模式（正则字符串数组）。默认空 `Vec` 表示匹配所有 URL。
多 Spider 共享队列时，引擎通过 `matches()` 判定 URL 应由哪个 Spider 处理。

#### `Spider::matches(&self, url: &str) -> bool`

默认实现：遍历 `patterns()`，任一正则匹配即返回 `true`；`patterns()` 为空时匹配所有 URL。
用户一般无需重写，重写 `patterns()` 即可。

#### `Spider::until(&self) -> Arc<dyn StopCondition>`

per-spider 终止策略。默认 `NeverStop`（由引擎 `max_pages` 兜底）。
返回 `Arc<dyn StopCondition>`，可组合使用 `and` / `or` / `not`。

### SpiderBuilder (`src/crawl/builder.rs`)

闭包式 Spider 构建，避免手写 trait impl：

```rust
use wisp::crawl::SpiderBuilder;
use wisp::crawl::stop::{MaxPages, Timeout};
use std::time::Duration;

let spider = SpiderBuilder::new("quotes")
    .start_urls(vec!["https://quotes.toscrape.com/"])
    .concurrent(10)
    .delay(Duration::from_millis(500))
    .patterns(vec![r"^https://quotes\.toscrape\.com/"])  // URL 路由
    .until(MaxPages(100).or(Timeout(Duration::from_secs(60))))  // 终止条件
    .parse(|resp| {
        let doc = resp.parse().unwrap();
        let items = doc.select(".quote").iter().map(|q| {
            serde_json::json!({ "text": q.select_one(".text").map(|n| n.text()) })
        }).collect();
        (items, vec![])
    })
    .build();
```

#### `SpiderBuilder::patterns(Vec<String>)`

设置 URL 路由模式。等价于在 Spider impl 中重写 `patterns()`。

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

### 多 Spider 共享队列引擎 (`src/crawl/mod.rs`)

`Engine` 支持单/多 Spider，多 Spider 共享连接池/缓存/代理池：

```rust
use wisp::crawl::Engine;

// 单 Spider 便捷构造
let engine = Engine::new(spider);

// 多 Spider 构造（共享队列路由）
let engine = Engine::spiders(vec![
    Box::new(list_spider),
    Box::new(detail_spider),
]);
```

#### `Engine::spiders(spiders: Vec<Box<dyn Spider>>) -> Self`

多 Spider 构造。引擎根据每个 Spider 的 `matches()` 将 URL 路由到对应 Spider。
每个 Spider 拥有独立的 `SpiderStats`、`ModeRuleEngine`、`until()` 终止策略。

### SpiderResponse (`src/crawl/mod.rs`)

爬取响应。Task 4 引入 `from_cache` 字段标记是否来自缓存（命中缓存不计入 `pages_crawled`）：

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

## 测试

```bash
cargo test --lib                    # lib 单元测试
cargo test --test stop_condition_test
cargo test --test builder_api_test
cargo test --test multi_spider_test
```

注意：`tests/real_scrape_test.rs`、`tests/cf_bypass_real_test.rs`、`tests/session_test.rs`
存在预先的 GBK 编码问题（非 UTF-8），需单独修复编码后才能编译。

## 构建

```bash
cargo build            # lib + bins
cargo build --release
```
