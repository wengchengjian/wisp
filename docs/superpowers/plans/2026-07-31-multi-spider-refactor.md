# 多 Spider 共享队列重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 wisp 从“单 Spider + 多 callback”重构为“Engine 基础设施 + 可选多 Spider 共享队列”，并让 `novel_crawler` 以多 handler 和多 spider 两种方式都能成功爬取 10 本书。

**Architecture:** Engine 负责全部传输/反拦截中间件和共享 item pipeline；Spider 只保留解析、follow、业务停止条件。多 Spider 模式下，多个 Spider 共享一个 scheduler，按 `Request.callback` 路由到第一个声明接受该 callback 的 Spider，每个 Spider 使用自己的 `until()` 和 stats。`Engine::run()` 保持单 Spider 语义，内部退化为 `run_many(vec![spider])`。

**Tech Stack:** Rust, tokio, async-trait, serde_json, moka, dashmap

## Global Constraints

- 不考虑向后兼容：删除旧 API 时直接删除，不保留 deprecated 别名。
- `SpiderBuilder` 不再暴露 `.middleware()` 和 `.pipeline()`。
- `HeadersMiddleware`、`UaRotationMiddleware`、`CookieChallengeMiddleware` 只由 `EngineBuilder` 配置。
- item pipeline 默认由 Engine 共享，所有 Spider 的 item 汇入同一链；`CrawlContext.spider_name` 用于分流。
- 验收：`cargo test --test novel_10_books_test` 必须通过；`cargo run --example novel_crawler -- --mode handler` 与 `--mode spider` 都成功爬取 10 本书。

---

## 文件结构

| 文件 | 责任 | 动作 |
|---|---|---|
| `src/crawl/middleware/builtin.rs` | `DefaultMiddlewareConfig` 增加 headers/UA/cookie 配置，移除无条件 UA/Cookie 注入 | 修改 |
| `src/crawl/runner.rs` | `Engine`/`EngineBuilder` 持有传输配置、共享 pipeline、自定义 middleware；实现 `run_many` | 修改 |
| `src/crawl/engine.rs` | `EngineState` 改为 `spiders: Vec<Arc<dyn Spider>>` + `all_stats: Vec<Arc<SpiderStats>>`；`process_request/process_response` 按请求路由 Spider/stats | 修改 |
| `src/crawl/mod.rs` | `Spider` trait 删除 `middlewares()`/`pipelines()`，新增 `accepts_callback()` | 修改 |
| `src/crawl/builder.rs` | 删除 `.middleware()`/`.pipeline()` 和对应字段；`ClosureSpider` 实现 `accepts_callback()` | 修改 |
| `src/fetcher/response.rs` | `Request` 增加 `spider: Option<usize>` 与 `.with_spider()` | 修改 |
| `examples/novel_crawler.rs` | 改成 `--mode handler|spider` 双实现，各爬 10 本书 | 修改 |
| `tests/multi_spider_routing_test.rs` | 多 Spider callback 路由 + per-spider until 单元/集成测试 | 新建 |
| `tests/novel_10_books_test.rs` | 本地三阶段小说站，多 handler 与多 spider 各爬 10 本书 | 新建 |

---

## Task 1: Engine 传输中间件配置化

**Files:**
- Modify: `src/crawl/middleware/builtin.rs`
- Modify: `src/crawl/runner.rs`
- Test: `src/crawl/runner.rs`（追加 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `UaRotationMiddleware::desktop()`, `HeadersMiddleware::new(...)`, `CookieChallengeMiddleware::default()`
- Produces: `EngineBuilder::headers(...)`, `EngineBuilder::ua_rotation(...)`, `EngineBuilder::cookie_challenge(...)`；`Engine` 新增 `pub(crate) headers: Vec<(String, String)>`、`pub(crate) ua_middleware: Option<UaRotationMiddleware>`、`pub(crate) cookie_challenge: bool`

- [ ] **Step 1: 修改 `DefaultMiddlewareConfig` 与 `default_middlewares`**

在 `src/crawl/middleware/builtin.rs` 中把 `DefaultMiddlewareConfig` 改为：

```rust
pub struct DefaultMiddlewareConfig {
    pub fetch_mode: FetchMode,
    pub delay: Duration,
    pub obey_robots: bool,
    pub cache_store: Option<Arc<dyn Store>>,
    pub http_client: Arc<Client>,
    pub robots_cache: Arc<RobotsCache>,
    pub rule_engine: Arc<Mutex<ModeRuleEngine>>,
    pub headers: Vec<(String, String)>,
    pub ua_middleware: Option<UaRotationMiddleware>,
    pub cookie_challenge: bool,
}
```

`default_middlewares` 中删除 `DomainFilterMiddleware`、`DepthLimitMiddleware` 以及无条件 `UaRotationMiddleware::desktop()`/`CookieChallengeMiddleware::default()`，替换为：

```rust
    // 2. 请求修改类（按 Engine 配置注入）
    if !cfg.headers.is_empty() {
        mws.push(Arc::new(HeadersMiddleware::new(cfg.headers)));
    }
    if let Some(ua) = cfg.ua_middleware {
        mws.push(Arc::new(ua));
    }

    // 3. 模式升级类（仅 Auto）
    if cfg.fetch_mode == FetchMode::Auto {
        mws.push(Arc::new(DynamicUpgradeMiddleware::new()));
        mws.push(Arc::new(StealthUpgradeMiddleware::new(cfg.rule_engine)));
    }

    // 4. 重试/挑战类（Cookie Challenge 按配置启用）
    if cfg.cookie_challenge {
        mws.push(Arc::new(CookieChallengeMiddleware::default()));
    }
    mws.push(Arc::new(BlockedRetryMiddleware::default()));
    mws.push(Arc::new(RetryMiddleware::new(Duration::from_millis(500))));
```

- [ ] **Step 2: 修改 `Engine`/`EngineBuilder` 字段与 Builder 方法**

在 `src/crawl/runner.rs` 的 `Engine` 结构体增加：

```rust
pub(crate) headers: Vec<(String, String)>,
pub(crate) ua_middleware: Option<crate::crawl::middleware::UaRotationMiddleware>,
pub(crate) cookie_challenge: bool,
```

`EngineBuilder` 增加三个方法：

```rust
    /// 设置固定请求头。
    pub fn headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    /// 设置 UA 轮换策略；不调用则不发 User-Agent。
    pub fn ua_rotation(mut self, ua: crate::crawl::middleware::UaRotationMiddleware) -> Self {
        self.ua_middleware = Some(ua);
        self
    }

    /// 是否启用 Cookie Challenge 自动处理。
    pub fn cookie_challenge(mut self, enabled: bool) -> Self {
        self.cookie_challenge = enabled;
        self
    }
```

`EngineBuilder::build()` 中把三个值写入 `Engine`。

- [ ] **Step 3: 写失败测试**

在 `src/crawl/runner.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_builder_transport_config() {
        let engine = Engine::infra()
            .headers(vec![("Accept".into(), "text/html".into())])
            .ua_rotation(crate::crawl::middleware::UaRotationMiddleware::desktop())
            .cookie_challenge(true)
            .build()
            .unwrap();
        assert_eq!(engine.headers.len(), 1);
        assert!(engine.ua_middleware.is_some());
        assert!(engine.cookie_challenge);
    }
}
```

- [ ] **Step 4: 运行测试确认失败**

Run: `cargo test --lib runner::tests::engine_builder_transport_config`
Expected: FAIL，编译错误提示 `headers`/`ua_middleware`/`cookie_challenge` 不存在。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --lib runner::tests::engine_builder_transport_config`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src/crawl/middleware/builtin.rs src/crawl/runner.rs
git commit -m "refactor(engine): transport middleware config on EngineBuilder"
```

---

## Task 2: 删除 SpiderBuilder middleware/pipeline

**Files:**
- Modify: `src/crawl/mod.rs`
- Modify: `src/crawl/builder.rs`
- Modify: `src/crawl/runner.rs`

**Interfaces:**
- Consumes: `Spider` trait 删除 `middlewares()`/`pipelines()` 默认方法
- Produces: `SpiderBuilder` 不再有 `.middleware()`/`.pipeline()`；Engine 链只由 Engine 配置构成

- [ ] **Step 1: 从 `Spider` trait 删除 middleware/pipeline 方法**

在 `src/crawl/mod.rs` 删除：

```rust
    /// 返回此 Spider 的中间件列表。默认为空。
    fn middlewares(&self) -> Vec<Arc<dyn middleware::Middleware>> {
        Vec::new()
    }
    /// 返回此 Spider 的 Item 管道列表。默认为空。
    fn pipelines(&self) -> Vec<Arc<dyn middleware::ItemPipeline>> {
        Vec::new()
    }
```

- [ ] **Step 2: 删除 `SpiderBuilder` 的 middleware/pipeline**

在 `src/crawl/builder.rs` 删除 `.middleware()`、`.pipeline()` 方法、`middlewares`/`pipelines` 字段，以及 `ClosureSpider` 中对应字段和 trait 实现。

- [ ] **Step 3: 修改 `run_inner` 的中间件链组装**

把 `src/crawl/runner.rs` 中链组装改为只使用 Engine 配置：

```rust
let mut chain = middleware::MiddlewareChain::new();
let defaults = middleware::builtin::default_middlewares(
    middleware::builtin::DefaultMiddlewareConfig {
        fetch_mode,
        delay: self.download_delay,
        obey_robots,
        cache_store: self.cache_store.clone(),
        http_client: mw_http_client,
        robots_cache: mw_robots_cache,
        rule_engine: rule_engine.clone(),
        headers: self.headers.clone(),
        ua_middleware: self.ua_middleware.clone(),
        cookie_challenge: self.cookie_challenge,
    },
);
chain.middlewares.extend(defaults);
chain.middlewares.extend(self.custom_middlewares.clone());
chain.pipelines = self.pipelines.clone();
chain.sort();
```

`Engine` 增加字段：

```rust
pub(crate) custom_middlewares: Vec<Arc<crate::crawl::middleware::Middleware>>,
pub(crate) pipelines: Vec<Arc<crate::crawl::middleware::ItemPipeline>>,
```

`EngineBuilder` 增加：

```rust
    pub fn middleware(mut self, mw: Arc<crate::crawl::middleware::Middleware>) -> Self {
        self.custom_middlewares.push(mw);
        self
    }

    pub fn pipeline(mut self, p: Arc<crate::crawl::middleware::ItemPipeline>) -> Self {
        self.pipelines.push(p);
        self
    }
```

- [ ] **Step 4: 跑 `cargo check --all-targets` 并修复编译错误**

Run: `cargo check --all-targets`
Expected: 编译通过；旧示例中 `.middleware(...)`/`.pipeline(...)` 全部按新 API 改写。

- [ ] **Step 5: 提交**

```bash
git add src/crawl/mod.rs src/crawl/builder.rs src/crawl/runner.rs examples
git commit -m "refactor(spider): remove middleware/pipeline from SpiderBuilder"
```

---

## Task 3: Request 与 Spider 增加 callback 路由能力

**Files:**
- Modify: `src/fetcher/response.rs`
- Modify: `src/crawl/mod.rs`
- Modify: `src/crawl/builder.rs`
- Test: `tests/multi_spider_routing_test.rs`（新建）

**Interfaces:**
- Consumes: `Request` 现有 `callback: Option<String>`
- Produces: `Request.spider: Option<usize>`、`Request::with_spider(usize)`、`Spider::accepts_callback(Option<&str>) -> bool`

- [ ] **Step 1: 给 `Request` 增加 spider 索引**

在 `src/fetcher/response.rs` 的 `Request` 结构体增加：

```rust
    /// 多 Spider 模式下已路由到的 Spider 索引；None 表示尚未路由。
    #[serde(default)]
    pub spider: Option<usize>,
```

`Request::get`/`post` 初始化为 `None`，并增加：

```rust
    /// 设置 Spider 索引（供 `Engine::run_many` 内部路由使用）。
    pub fn with_spider(mut self, idx: usize) -> Self {
        self.spider = Some(idx);
        self
    }
```

- [ ] **Step 2: `Spider` trait 增加 `accepts_callback`**

在 `src/crawl/mod.rs` 的 `Spider` trait 中增加默认方法：

```rust
    /// 该 Spider 是否接受指定 callback 的请求。
    ///
    /// 多 Spider 模式下，Engine 从共享队列取出请求后按此方法路由。
    /// 默认实现接受所有请求；`ClosureSpider` 根据注册的 handler 覆盖。
    fn accepts_callback(&self, _callback: Option<&str>) -> bool {
        true
    }
```

- [ ] **Step 3: `ClosureSpider` 实现 callback 路由**

在 `src/crawl/builder.rs` 的 `impl Spider for ClosureSpider` 增加：

```rust
    fn accepts_callback(&self, callback: Option<&str>) -> bool {
        match callback {
            None => self.handlers.contains_key("default"),
            Some(cb) => {
                self.handlers.contains_key(cb) || self.handlers.contains_key("default")
            }
        }
    }
```

- [ ] **Step 4: 写路由单元测试**

新建 `tests/multi_spider_routing_test.rs`：

```rust
use serde_json::json;
use wisp::crawl::{Request, SpiderBuilder};

#[test]
fn closure_spider_accepts_only_owned_callbacks() {
    let spider = SpiderBuilder::new("detail")
        .on_page("detail", |page| page)
        .build();

    assert!(!spider.accepts_callback(None));
    assert!(spider.accepts_callback(Some("detail")));
    assert!(!spider.accepts_callback(Some("chapter")));

    let home = SpiderBuilder::new("home")
        .on_page("default", |page| page)
        .build();
    assert!(home.accepts_callback(None));
    assert!(home.accepts_callback(Some("unknown"))); // 回退 default

    let req = Request::get("https://example.com/book/1").with_callback("detail");
    assert!(spider.accepts_callback(req.callback.as_deref()));
}
```

- [ ] **Step 5: 运行测试**

Run: `cargo test --test multi_spider_routing_test`
Expected: 2 个断言全过。

- [ ] **Step 6: 提交**

```bash
git add src/fetcher/response.rs src/crawl/mod.rs src/crawl/builder.rs tests/multi_spider_routing_test.rs
git commit -m "feat(spider): callback ownership routing for multi-spider"
```

---

## Task 4: Engine::run_many 共享队列实现

**Files:**
- Modify: `src/crawl/engine.rs`
- Modify: `src/crawl/runner.rs`
- Test: `tests/multi_spider_routing_test.rs`

**Interfaces:**
- Consumes: `Request.spider`、`Spider::accepts_callback`、`Spider::until`
- Produces: `Engine::run_many(Vec<S>) -> Result<(Vec<CrawlStats>, Vec<Value>)>`；`Engine::run` 改为调用 `run_many` 并取第一份 stats

- [ ] **Step 1: 改造 `EngineState` 为多 Spider**

在 `src/crawl/engine.rs` 中把：

```rust
pub spider: Arc<dyn Spider>,
pub stats: Arc<SpiderStats>,
```

替换为：

```rust
pub spiders: Vec<Arc<dyn Spider>>,
pub all_stats: Vec<Arc<SpiderStats>>,
```

并增加：

```rust
impl EngineState {
    pub fn spider_index_for(&self, req: &Request) -> Option<usize> {
        if let Some(idx) = req.spider {
            return self.spiders.get(idx).map(|_| idx);
        }
        self.spiders
            .iter()
            .position(|s| s.accepts_callback(req.callback.as_deref()))
    }

    pub fn spider_for(&self, req: &Request) -> Option<Arc<dyn Spider>> {
        self.spider_index_for(req).map(|i| Arc::clone(&self.spiders[i]))
    }

    pub fn stats_for(&self, req: &Request) -> Option<Arc<SpiderStats>> {
        self.spider_index_for(req)
            .map(|i| Arc::clone(&self.all_stats[i]))
    }
}
```

- [ ] **Step 2: `process_request` 按请求取 Spider/stats**

把 `src/crawl/engine.rs` 的 `process_request` 开头改为：

```rust
    let spider = match ctx.state.spider_for(&req) {
        Some(s) => s,
        None => return None,
    };
    let stats = ctx.state.stats_for(&req).expect("spider stats");

    // 域名与深度是 Spider 业务策略，保留在 Spider 上。
    if !spider.allowed_domains().is_empty() {
        let host = url::Url::parse(&req.url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_default();
        let ok = spider.allowed_domains().iter().any(|d| {
            host == *d || host.ends_with(&format!(".{d}"))
        });
        if !ok {
            return None;
        }
    }
    if req.depth > spider.max_depth() {
        return None;
    }
```

后续所有 `ctx.state.stats` 改为局部 `stats`；`spider.on_error(...)` 改为局部 `spider`。

- [ ] **Step 3: `process_response` 按请求取 Spider/stats**

在 `src/crawl/engine.rs` 的 `process_response` 开头改为：

```rust
    let spider = match ctx.state.spider_for(&resp.request) {
        Some(s) => s,
        None => return,
    };
    let stats = ctx.state.stats_for(&resp.request).expect("spider stats");
```

后续所有 `ctx.state.stats`/`ctx.state.spider` 改为局部变量。

- [ ] **Step 4: `run_many` 主循环**

在 `src/crawl/runner.rs` 新增：

```rust
    pub async fn run_many<S: Spider + 'static>(
        &self,
        spiders: Vec<S>,
    ) -> Result<(Vec<CrawlStats>, Vec<Value>)> {
        let spiders: Vec<Arc<dyn Spider>> = spiders.into_iter().map(Arc::new).collect();
        let items: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let stats = self
            .run_inner_many(spiders, None, items.clone())
            .await?;
        Ok((stats, items.lock().await.clone()))
    }

    pub async fn run<S: Spider + 'static>(
        &self,
        spider: S,
    ) -> Result<(CrawlStats, Vec<Value>)> {
        let (mut stats, items) = self.run_many(vec![spider]).await?;
        Ok((stats.remove(0), items))
    }
```

把原 `run_inner` 改名为 `run_inner_many(spiders: Vec<Arc<dyn Spider>>, ...) -> Result<Vec<CrawlStats>>`，内部做以下修改：

1. `let all_stats: Vec<Arc<SpiderStats>> = spiders.iter().map(|_| Arc::new(SpiderStats::new())).collect();`
2. 种子请求：`for (i, spider) in spiders.iter().enumerate() { for url in spider.start_urls() { sched.push(Request::get(&url).with_spider(i)).await; } }`
3. 调用 `for spider in &spiders { spider.on_start().await; }`
4. 每次 pop 后调用 `ctx.state.spider_index_for(&req)`；找不到则丢弃并 `continue`。
5. 对该 Spider 构造 `StopContext` 并检查 `until()`；已停止则丢弃该请求并 `continue`。
6. 派发前把 `req.spider = Some(idx)`。
7. 引擎级 `max_pages` 使用所有 stats 的 pages 总和。
8. 本计划暂时不持久化多 Spider checkpoint；单 Spider 的 `run` 也随 `run_many` 一起跳过，后续单独计划恢复。
9. 结束时 `for spider in &spiders { spider.on_close().await; }`，返回 `Vec<CrawlStats>`。

- [ ] **Step 5: 写 per-spider until 集成测试**

在 `tests/multi_spider_routing_test.rs` 追加：

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wisp::crawl::stop::MaxPages;
use wisp::crawl::{Engine, SpiderBuilder};
use wisp::fetcher::FetchMode;

async fn spawn_stage_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let body = "<html><body><a href=\"/book/1\">1</a><a href=\"/book/2\">2</a><a href=\"/book/3\">3</a></body></html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn detail_spider_until_does_not_block_chapter_spider() {
    let base = spawn_stage_server().await;
    let home = SpiderBuilder::new("home")
        .start_urls(vec![base.clone()])
        .on_page("default", |mut page| {
            page.follow_links(&["a"], "detail", |_i, a| {
                serde_json::json!({ "title": a.text().trim() })
            });
            page
        })
        .build();
    let detail = SpiderBuilder::new("detail")
        .on_page("detail", |mut page| {
            page.follow_links(&["a"], "chapter", |_i, a| {
                serde_json::json!({ "title": page.meta_str("title") })
            });
            page
        })
        .until(MaxPages(2))
        .build();
    let chapter = SpiderBuilder::new("chapter")
        .on_page("chapter", |mut page| {
            page.item(serde_json::json!({ "title": page.meta_str("title") }));
            page
        })
        .build();

    let engine = Engine::infra()
        .max_concurrent(1)
        .max_pages(100)
        .obey_robots(false)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap();

    let (stats, items) = engine
        .run_many(vec![home, detail, chapter])
        .await
        .unwrap();
    assert_eq!(stats[0].pages_crawled, 1, "home 只爬首页");
    assert_eq!(stats[1].pages_crawled, 2, "detail 只爬 2 个详情");
    assert_eq!(items.len(), 2, "chapter 应产出 2 个 item");
}
```

- [ ] **Step 6: 运行测试**

Run: `cargo test --test multi_spider_routing_test`
Expected: 全部 PASS。

- [ ] **Step 7: 跑全量编译并提交**

Run: `cargo check --all-targets`
Expected: 编译通过。

```bash
git add src/crawl/engine.rs src/crawl/runner.rs tests/multi_spider_routing_test.rs
git commit -m "feat(engine): shared-queue run_many with per-spider until"
```

---

## Task 5: Engine 共享 pipeline 与示例改造

**Files:**
- Modify: `examples/novel_crawler.rs`
- Test: `tests/novel_10_books_test.rs`（新建）

**Interfaces:**
- Consumes: `EngineBuilder::pipeline`、`SpiderBuilder::on_page`、`Page::follow_links_n`
- Produces: `examples/novel_crawler.rs --mode handler|spider`；本地小说站验收测试

- [ ] **Step 1: 把 `examples/novel_crawler.rs` 改成双模式**

`main` 解析 `--mode`，并实现两个构建函数：

```rust
fn build_handler_spider(base: &str) -> impl wisp::crawl::Spider {
    use wisp::crawl::stop::MaxPages;
    use wisp::crawl::SpiderBuilder;

    SpiderBuilder::new("novel-handler")
        .start_urls(vec![base.to_string()])
        .on_page("default", |mut page| {
            page.follow_links_n(&[".book a"], "detail", 10, |_i, a| {
                serde_json::json!({ "title": a.text().trim(), "author": "" })
            });
            page
        })
        .on_page("detail", |mut page| {
            page.follow_links(&[".chapter a"], "chapter", |i, a| {
                serde_json::json!({
                    "title": page.meta_str("title"),
                    "author": page.meta_str("author"),
                    "chapter_title": a.text().trim(),
                    "chapter_index": i,
                })
            });
            page
        })
        .on_page("chapter", |mut page| {
            page.item(serde_json::json!({
                "title": page.meta_str("title"),
                "author": page.meta_str("author"),
                "chapter_title": page.meta_str("chapter_title"),
                "chapter_index": page.meta_u64("chapter_index"),
            }));
            page
        })
        .until(MaxPages(100))
        .build()
}

fn build_multi_spiders(base: &str) -> Vec<wisp::crawl::ClosureSpider> {
    use wisp::crawl::stop::MaxPages;
    use wisp::crawl::SpiderBuilder;

    let home = SpiderBuilder::new("home")
        .start_urls(vec![base.to_string()])
        .on_page("default", |mut page| {
            page.follow_links(&[".book a"], "detail", |_i, a| {
                serde_json::json!({ "title": a.text().trim(), "author": "" })
            });
            page
        })
        .build();

    let detail = SpiderBuilder::new("detail")
        .on_page("detail", |mut page| {
            page.follow_links(&[".chapter a"], "chapter", |i, a| {
                serde_json::json!({
                    "title": page.meta_str("title"),
                    "author": page.meta_str("author"),
                    "chapter_title": a.text().trim(),
                    "chapter_index": i,
                })
            });
            page
        })
        .until(MaxPages(10))
        .build();

    let chapter = SpiderBuilder::new("chapter")
        .on_page("chapter", |mut page| {
            page.item(serde_json::json!({
                "title": page.meta_str("title"),
                "author": page.meta_str("author"),
                "chapter_title": page.meta_str("chapter_title"),
                "chapter_index": page.meta_u64("chapter_index"),
            }));
            page
        })
        .build();

    vec![home, detail, chapter]
}
```

`main` 中统一构建 Engine：

```rust
let engine = Engine::infra()
    .proxy("http://127.0.0.1:7897")
    .headers(vec![
        ("Accept".into(), "text/html,application/xhtml+xml".into()),
        ("Accept-Language".into(), "zh-CN,zh;q=0.9".into()),
    ])
    .ua_rotation(wisp::crawl::middleware::UaRotationMiddleware::desktop())
    .cookie_challenge(true)
    .pipeline(Arc::new(JsonlWriterPipeline::new(output)))
    .build()?;
```

`--mode handler` 调用 `engine.run(spider).await`；`--mode spider` 调用 `engine.run_many(spiders).await`。两种模式统计后打印“书籍数 >= 10”。

- [ ] **Step 2: 写本地小说站 10 本书验收测试**

新建 `tests/novel_10_books_test.rs`：

```rust
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wisp::crawl::stop::MaxPages;
use wisp::crawl::{Engine, SpiderBuilder};
use wisp::fetcher::FetchMode;

async fn spawn_novel_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let path = String::from_utf8_lossy(&buf)
                    .lines()
                    .next()
                    .unwrap_or("GET / HTTP/1.1")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/");
                let body = if path == "/" {
                    (1..=12)
                        .map(|i| format!(r#"<a class="book" href="/book/{i}">Book {i}</a>"#))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else if path.starts_with("/book/") {
                    r#"<a class="chapter" href="/chapter/1">Ch1</a><a class="chapter" href="/chapter/2">Ch2</a>"#.to_string()
                } else {
                    "<p>content</p>".to_string()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{}", addr)
}

fn engine() -> wisp::Engine {
    Engine::infra()
        .max_concurrent(1)
        .max_pages(100)
        .obey_robots(false)
        .fetch_mode(FetchMode::Http)
        .build()
        .unwrap()
}

fn assert_ten_books(items: &[serde_json::Value]) {
    let books: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|v| v["title"].as_str())
        .collect();
    assert_eq!(books.len(), 10, "应爬取 10 本书，实际 {}", books.len());
}

#[tokio::test]
async fn handler_mode_crawls_ten_books() {
    let base = spawn_novel_server().await;
    let spider = SpiderBuilder::new("handler")
        .start_urls(vec![base.clone()])
        .on_page("default", |mut page| {
            page.follow_links_n(&[".book a"], "detail", 10, |_i, a| {
                serde_json::json!({ "title": a.text().trim() })
            });
            page
        })
        .on_page("detail", |mut page| {
            page.follow_links(&[".chapter a"], "chapter", |_i, a| {
                serde_json::json!({ "title": page.meta_str("title"), "chapter_title": a.text().trim() })
            });
            page
        })
        .on_page("chapter", |mut page| {
            page.item(serde_json::json!({ "title": page.meta_str("title") }));
            page
        })
        .until(MaxPages(100))
        .build();
    let (_, items) = engine().run(spider).await.unwrap();
    assert_ten_books(&items);
}

#[tokio::test]
async fn spider_mode_crawls_ten_books() {
    let base = spawn_novel_server().await;
    let home = SpiderBuilder::new("home")
        .start_urls(vec![base.clone()])
        .on_page("default", |mut page| {
            page.follow_links(&[".book a"], "detail", |_i, a| {
                serde_json::json!({ "title": a.text().trim() })
            });
            page
        })
        .build();
    let detail = SpiderBuilder::new("detail")
        .on_page("detail", |mut page| {
            page.follow_links(&[".chapter a"], "chapter", |_i, a| {
                serde_json::json!({ "title": page.meta_str("title"), "chapter_title": a.text().trim() })
            });
            page
        })
        .until(MaxPages(10))
        .build();
    let chapter = SpiderBuilder::new("chapter")
        .on_page("chapter", |mut page| {
            page.item(serde_json::json!({ "title": page.meta_str("title") }));
            page
        })
        .build();
    let (_, items) = engine().run_many(vec![home, detail, chapter]).await.unwrap();
    assert_ten_books(&items);
}
```

- [ ] **Step 3: 运行验收测试**

Run: `cargo test --test novel_10_books_test`
Expected: 2 个测试全 PASS。

- [ ] **Step 4: 真实站手工验收**

Run:

```bash
cargo run --example novel_crawler -- --mode handler
cargo run --example novel_crawler -- --mode spider
```

Expected: 两种模式都输出至少 10 本书的章节条目，退出码 0。

- [ ] **Step 5: 提交**

```bash
git add examples/novel_crawler.rs tests/novel_10_books_test.rs
git commit -m "feat(example): novel_crawler dual mode handler/spider with 10-book acceptance"
```

---

## Task 6: 回归清理

**Files:**
- Modify: `tests/cf_bypass_real_test.rs`
- Modify: `src/crawl/middleware/mod.rs`
- Modify: `README.md`

- [ ] **Step 1: 全局搜索旧 API**

Run: `rg -n "SpiderBuilder.*middleware|\.pipeline\(|\.middleware\(" examples tests src`
Expected: 剩余结果只有：

```text
examples/novel_crawler.rs（Task 5 已改写）
tests/cf_bypass_real_test.rs:201  `.middleware(ProxyInjectionMiddleware::new(pool))`
src/crawl/middleware/mod.rs:17-18 文档示例
```

- [ ] **Step 2: 修复剩余引用**

在 `tests/cf_bypass_real_test.rs` 中删除 `SpiderBuilder::middleware(ProxyInjectionMiddleware::new(pool))`，改为在 `EngineBuilder` 上配置：

```rust
let engine = Engine::infra()
    .middleware(Arc::new(wisp::crawl::middleware::ProxyInjectionMiddleware::new(pool)))
    .build()
    .unwrap();
```

`ProxyInjectionMiddleware` 依赖的 `pool` 若来自 `SpiderBuilder` 闭包，则改为测试函数外层构造并 move 进 Engine。

更新 `src/crawl/middleware/mod.rs` 顶部文档，把示例改为：

```rust
use wisp::crawl::middleware::{UaRotationMiddleware, RetryMiddleware};

let engine = Engine::infra()
    .ua_rotation(UaRotationMiddleware::desktop())
    .build()?;
```

- [ ] **Step 3: 更新 README**

在 README 的多 Spider 小节补充：

```text
多 handler：一个 Spider 通过 callback label 分发阶段。
多 spider：Engine::run_many 使用共享队列，按 callback 路由，每个 Spider 独立 until/stats。
```

- [ ] **Step 4: 全量验证**

Run: `cargo test --lib && cargo test --test novel_10_books_test --test multi_spider_routing_test && cargo check --all-targets`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git add tests/cf_bypass_real_test.rs src/crawl/middleware/mod.rs README.md
git commit -m "chore: migrate examples/tests to engine-owned middleware and shared pipelines"
```

---

## 验收清单

- [ ] `cargo test --test novel_10_books_test` 通过，两种实现都成功爬取 10 本书。
- [ ] `cargo run --example novel_crawler -- --mode handler` 成功。
- [ ] `cargo run --example novel_crawler -- --mode spider` 成功。
- [ ] `cargo test --lib` 通过。
- [ ] `cargo check --all-targets` 通过。
- [ ] `SpiderBuilder` 不再有 middleware/pipeline API。
- [ ] Engine 默认不再无条件注入 UA/Cookie Challenge。
