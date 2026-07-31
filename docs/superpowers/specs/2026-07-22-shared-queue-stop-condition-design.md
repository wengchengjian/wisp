# 共享队列 + 路由 + 终止条件 设计

**日期**：2026-07-22
**状态**：设计阶段
**关联**：Stage 4 后续改进

## 1. 背景与动机

### 1.1 当前架构局限

wisp 当前多 Spider 架构是"各自独立队列"：

- `Engine::add_spider(A).add_spider(B).run()` 在 [run_with_sender](file:///f:/project/wisp/src/crawl/mod.rs#L378) 中为每个 Spider 启动独立 task + 独立 scheduler + 独立 follow channel
- follow URL 回到自己的 scheduler（[engine.rs:242-244](file:///f:/project/wisp/src/crawl/engine.rs#L242-L244)）
- Spider A 产出的详情页 URL 无法被 Spider B 消费

### 1.2 `max_pages` 的根本问题

`max_pages` 把"页数"这个统计概念当作了控制流概念：

- [mod.rs:549](file:///f:/project/wisp/src/crawl/mod.rs#L549) 用全局单一 `stats_pages` 判断终止
- 无法表达"列表页 50 页停，详情页继续"这种分类终止
- 多阶段爬取场景（列表 → 详情 → 章节）下无意义

### 1.3 缓存命中误统计 bug

[engine.rs:114](file:///f:/project/wisp/src/crawl/engine.rs#L114) 和 [engine.rs:209](file:///f:/project/wisp/src/crawl/engine.rs#L209) 命中缓存时也走 `process_response`，导致 `pages_crawled` 把缓存命中也算进去，统计失真。

### 1.4 目标模型

爬虫本质是 URL 消费者：

```
共享 URL 队列
    ↓
Spider A (matches list URL)   Spider B (matches detail URL)
  until: pages >= 50            until: NeverStop
  ↓ 产出详情 URL                 ↓ 消费详情
  └→ 回到共享队列 → Spider B
```

- 多 Spider 从同一队列取 URL
- `matches(url)` 决定 URL 归属
- `until()` 决定单个 Spider 何时停止消费
- 上游 Spider 停止 → 不再产出下游 URL → 下游 Spider 自然枯竭

## 2. 设计方案

### 2.1 三层职责划分

```
┌─────────────────────────────────────────┐
│ EngineContext (引擎持有，全局共享)        │
│ - client, sched(共享), domain_sems,     │
│   robots_cache, proxy_pool, cache_store, │
│   request_cache, abort_flag, tx, start   │
│ - spiders: Vec<Arc<dyn Spider>>          │
│ - stats: Vec<Arc<SpiderStats>>           │ ← per-spider 统计
│ - global_in_flight: AtomicUsize          │
└─────────────────────────────────────────┘
         │
         │  路由：spiders[i].matches(url)  ← Spider trait 方法
         │  检查：spiders[i].until().should_stop(&stop_ctx)
         │
         ↓
┌─────────────────────────────────────────┐
│ process_request(engine_ctx, spider,     │
│                 stats, req)              │
│ - spider.allowed_domains/max_depth/      │
│   fetch_mode/fetcher_config 直接调方法    │
│ - stats.pages += 1                       │ ← per-spider 统计
└─────────────────────────────────────────┘
         │
         │  until 闭包读：
         ↓
┌─────────────────────────────────────────┐
│ StopContext (只读快照)                    │
│ - pages, items, errors                  │
│ - in_flight, elapsed, queue_size        │
└─────────────────────────────────────────┘
```

**关键原则**：SpiderContext 由 Spider 持有自己的状态，引擎不单独暴露 SpiderContext 结构。引擎维护 `Vec<Arc<dyn Spider>>` + `Vec<Arc<SpiderStats>>`，路由时只选 Spider，不构造中间结构。

### 2.2 不单独暴露 SpiderContext

**Spider trait 持有自己的状态，引擎不构造也不暴露 SpiderContext 结构**。引擎维护 `Vec<Arc<dyn Spider>>` + `Vec<Arc<SpiderStats>>`，路由时只选哪个 Spider，调用其方法（`matches`、`parse`、`until`、`allowed_domains` 等）。Spider 的配置（allowed_domains、max_depth、fetch_mode 等）通过 trait 方法暴露，不由引擎包装成中间结构。

### 2.3 字段归属表

| 字段 | 当前位置 | 归属层 | 说明 |
|---|---|---|---|
| client | EngineContext | Engine | 共享连接池 |
| sched | EngineContext | **Engine** | **改为共享队列** |
| domain_sems | EngineContext | Engine | 共享域名信号量 |
| robots_cache | EngineContext | Engine | 共享 |
| proxy_pool | EngineContext | Engine | 共享 |
| cache_store | EngineContext | Engine | 共享 |
| request_cache | EngineContext | Engine | 共享 |
| stats_items | EngineContext | **SpiderStats** | per-spider 统计 |
| stats_pages | EngineContext | **SpiderStats** | per-spider 统计 |
| stats_errors | EngineContext | **SpiderStats** | per-spider 统计 |
| stats_blocked | EngineContext | SpiderStats | per-spider |
| stats_retries | EngineContext | SpiderStats | per-spider |
| stats_offsite | EngineContext | SpiderStats | per-spider |
| stats_cache_hits | EngineContext | SpiderStats | per-spider |
| stats_status_codes | EngineContext | SpiderStats | per-spider |
| in_flight | EngineContext | **SpiderStats** | per-spider in-flight |
| global_in_flight | 无 | **Engine** | 新增，全局在飞数 |
| allowed | EngineContext | Spider trait 方法 | per-spider 域名白名单 |
| max_depth | EngineContext | Spider trait 方法 | per-spider |
| max_concurrent | EngineContext | Spider trait 方法 | per-spider |
| obey_robots | EngineContext | Spider trait 方法 | per-spider |
| fetch_mode | EngineContext | Spider trait 方法 | per-spider |
| fetcher_config | EngineContext | Spider trait 方法 | per-spider |
| rule_engine | EngineContext | Engine（per-spider auto 规则） | 改为引擎持有多个 |
| auto_exclude | EngineContext | Spider trait 方法 | per-spider |
| abort_flag | EngineContext | Engine | 全局停止 |
| tx | EngineContext | Engine | 全局事件 |
| start | EngineContext | Engine | 全局开始时间 |
| spider | EngineContext | **不在 ctx，路由时传参** | per-spider |

### 2.4 数据流

```
共享 scheduler.pop() → URL
    ↓
遍历 spiders.find(|s| s.matches(url))   ← matches 默认遍历 patterns()
    ↓ 找到
检查 spider.until().should_stop(&StopContext)
    ↓ should_stop=false
process_request(engine_ctx, spider, &stats[i], req)
    ↓
fetch_with_retry → process_response
    ↓
spider.parse(resp) → (items, follows)
    ↓ follows
follow_tx.send(f) → 回到共享 scheduler
    ↓
stats[i].pages += 1（per-spider SpiderStats）
    ↓ 无匹配
stats_dropped += 1（新增统计）
```

## 3. 接口设计

### 3.1 Spider trait

```rust
pub trait Spider: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn start_urls(&self) -> Vec<String>;
    
    /// URL 匹配。默认实现：遍历 patterns()，任一匹配即返回 true。
    fn matches(&self, url: &str) -> bool {
        self.patterns().iter().any(|p| {
            regex::Regex::new(p).map(|re| re.is_match(url)).unwrap_or(false)
        })
    }
    
    /// URL 匹配模式（字符串数组，内部自动编译为正则）。默认空 Vec（匹配所有）。
    fn patterns(&self) -> Vec<String> { Vec::new() }
    
    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>);
    
    /// 终止条件。默认永不停止（由引擎 max_pages 兜底）。
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(NeverStop)
    }
    
    // ... 其他可选方法保持不变（allowed_domains, max_depth, fetcher_config 等）
}
```

### 3.2 StopCondition trait

```rust
pub trait StopCondition: Send + Sync {
    fn should_stop(&self, ctx: &StopContext) -> bool;
    
    fn and<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>
    where Self: Sized + 'static
    {
        Arc::new(And { a: Arc::new(self), b: Arc::new(other) })
    }
    fn or<C: StopCondition + 'static>(self, other: C) -> Arc<dyn StopCondition>
    where Self: Sized + 'static
    {
        Arc::new(Or { a: Arc::new(self), b: Arc::new(other) })
    }
    fn not(self) -> Arc<dyn StopCondition>
    where Self: Sized + 'static
    {
        Arc::new(Not { inner: Arc::new(self) })
    }
}
```

### 3.3 原子策略

```rust
pub struct MaxPages(pub usize);
impl StopCondition for MaxPages {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.pages >= self.0
    }
}

pub struct MaxItems(pub usize);
impl StopCondition for MaxItems {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.items >= self.0
    }
}

pub struct MaxErrors(pub usize);
impl StopCondition for MaxErrors {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.errors >= self.0
    }
}

pub struct Timeout(pub Duration);
impl StopCondition for Timeout {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        ctx.elapsed >= self.0
    }
}

pub struct NeverStop;
impl StopCondition for NeverStop {
    fn should_stop(&self, _ctx: &StopContext) -> bool { false }
}

// 便捷：闭包转 StopCondition
pub struct FnStopCondition<F: Fn(&StopContext) -> bool + Send + Sync>(pub F);
impl<F: Fn(&StopContext) -> bool + Send + Sync> StopCondition for FnStopCondition<F> {
    fn should_stop(&self, ctx: &StopContext) -> bool { (self.0)(ctx) }
}
```

### 3.4 组合策略

```rust
struct And { a: Arc<dyn StopCondition>, b: Arc<dyn StopCondition> }
impl StopCondition for And {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        self.a.should_stop(ctx) && self.b.should_stop(ctx)
    }
}

struct Or { a: Arc<dyn StopCondition>, b: Arc<dyn StopCondition> }
impl StopCondition for Or {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        self.a.should_stop(ctx) || self.b.should_stop(ctx)
    }
}

struct Not { inner: Arc<dyn StopCondition> }
impl StopCondition for Not {
    fn should_stop(&self, ctx: &StopContext) -> bool {
        !self.inner.should_stop(ctx)
    }
}
```

### 3.5 StopContext

```rust
pub struct StopContext {
    pub pages: usize,           // 该 Spider 已爬页数
    pub items: usize,           // 该 Spider 已产 item 数
    pub errors: usize,          // 该 Spider 错误数
    pub in_flight: usize,       // 该 Spider 在飞请求数
    pub elapsed: Duration,      // 该 Spider 已运行时长
    pub queue_size: usize,      // 共享队列剩余请求数
}
```

### 3.6 SpiderStats

```rust
pub struct SpiderStats {
    pub pages: AtomicUsize,
    pub items: AtomicUsize,
    pub errors: AtomicUsize,
    pub blocked: AtomicUsize,
    pub retries: AtomicUsize,
    pub offsite: AtomicUsize,
    pub cache_hits: AtomicUsize,
    pub in_flight: AtomicUsize,
    pub status_codes: Mutex<HashMap<u16, usize>>,
}
```

### 3.7 EngineContext

```rust
pub struct EngineContext {
    pub client: Arc<Client>,
    pub sched: Arc<Scheduler>,                              // 共享队列
    pub robots_cache: Arc<Mutex<RobotsCache>>,
    pub domain_sems: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    pub proxy_pool: Option<Arc<ProxyPool>>,
    pub cache_store: Option<Arc<Store>>,
    pub request_cache: Option<RequestCache>,
    pub global_in_flight: Arc<AtomicUsize>,                 // 新增
    pub abort_flag: Arc<AtomicBool>,
    pub tx: Option<Sender<CrawlEvent>>,
    pub start: Instant,
}
```

### 3.8 FunctionSpider 与 SpiderBuilder

提供闭包式定义，内部实现 `Spider` trait：

```rust
pub struct FunctionSpider {
    name: String,
    start_urls: Vec<String>,
    patterns: Vec<String>,   // 字符串数组，matches() 内部编译为正则
    parse_fn: Box<dyn Fn(SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) + Send + Sync>,
    until_cond: Arc<dyn StopCondition>,
    // 其他可选配置
}

impl Spider for FunctionSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
    fn patterns(&self) -> Vec<String> { self.patterns.clone() }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (self.parse_fn)(resp)
    }
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::clone(&self.until_cond)
    }
}

pub struct SpiderBuilder { /* fields */ }
impl SpiderBuilder {
    pub fn patterns(mut self, patterns: Vec<String>) -> Self { ... }
    pub fn parse<F>(mut self, f: F) -> Self
    where F: Fn(SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) + Send + Sync + 'static { ... }
    pub fn until<C: StopCondition + 'static>(mut self, cond: C) -> Self { ... }
    pub fn build(self) -> FunctionSpider { ... }
}
```

**`until` 返回 `Arc<dyn StopCondition>`**：`Arc` 可 clone，`FunctionSpider` 持有 `Arc<dyn StopCondition>`，每次 `until()` 调用 `Arc::clone` 即可，无 clone 问题。

### 3.9 用法示例

**trait 方式（结构体）**：

```rust
struct ListSpider { max_page: usize }

impl Spider for ListSpider {
    fn name(&self) -> &str { "list" }
    fn start_urls(&self) -> Vec<String> { vec!["https://example.com/list/1".into()] }
    fn patterns(&self) -> Vec<String> { vec![r"example\.com/list/\d+".into()] }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        // 提取详情 URL
        (items, detail_urls)
    }
    fn until(&self) -> Arc<dyn StopCondition> {
        Arc::new(MaxPages(self.max_page))
    }
}

struct DetailSpider;

impl Spider for DetailSpider {
    fn name(&self) -> &str { "detail" }
    fn start_urls(&self) -> Vec<String> { vec![] }  // 不主动启动，等 ListSpider 产出
    fn patterns(&self) -> Vec<String> { vec![r"example\.com/detail/\d+".into()] }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (items, vec![])  // 不再 follow
    }
    // until 用默认 NeverStop，受限于上游 ListSpider
}

Engine::new()
    .add_spider(ListSpider { max_page: 50 })
    .add_spider(DetailSpider)
    .max_pages(10000)  // 引擎级兜底
    .run().await?;
```

**闭包方式（SpiderBuilder）**：

```rust
Engine::new()
    .spider(
        SpiderBuilder::new()
            .name("list")
            .start_urls(vec!["https://example.com/list/1".into()])
            .patterns(vec![r"example\.com/list/\d+".into()])
            .parse(|resp| { /* 返回 items + detail_urls */ })
            .until(MaxPages(50))
            .build()
    )
    .spider(
        SpiderBuilder::new()
            .name("detail")
            .patterns(vec![r"example\.com/detail/\d+".into()])
            .parse(|resp| { /* 返回 items */ })
            .build()
    )
    .run().await?;
```

## 4. 关键改动点

### 4.1 scheduler 共享化

- `EngineContext.sched` 从 per-spider 改为共享
- 所有 Spider 的 follow URL 推到同一共享队列
- `Scheduler` 已是 `Arc<Mutex<...>>` + `Clone`，支持共享

### 4.2 follow_tx 共享化

- 当前 [engine.rs:242-244](file:///f:/project/wisp/src/crawl/engine.rs#L242-L244) 把 follow 推回自己的 scheduler
- 改为推回共享 `follow_tx` → 共享 scheduler

### 4.3 路由逻辑

- 从共享队列取出 URL 后，遍历 `spiders` 找 `matches(url) == true` 的第一个
- 无匹配则丢弃（新增 `stats_dropped` 统计）
- `matches` 默认遍历 `patterns()`，任一正则匹配即处理

### 4.4 until 检查

- 每次派发前，构造 `StopContext` 快照
- 调用 `spider.until().should_stop(&stop_ctx)`
- 返回 true 则该 Spider 不再被路由（跳过，不丢弃 URL，留给下一个匹配的 Spider）
- 若所有匹配的 Spider 都 until=true，URL 丢弃

### 4.5 引擎退出条件

- 共享队列空 + 全局 `global_in_flight == 0` → 结束
- `abort_flag == true` → 立即结束
- 保留 `EngineBuilder::max_pages` 作为引擎级兜底

### 4.6 修复缓存命中误统计 bug

- `SpiderResponse` 新增 `from_cache: bool` 字段
- [engine.rs:218](file:///f:/project/wisp/src/crawl/engine.rs#L218) `process_response` 中 `if !resp.from_cache { stats_pages += 1 }`

### 4.7 per-spider 统计

- 引擎维护 `Vec<Arc<SpiderStats>>`，与 `Vec<Arc<dyn Spider>>` 一一对应
- `process_request` 接收 `&Arc<SpiderStats>` 参数
- `CrawlStats` 聚合所有 SpiderStats（或返回 per-spider 数组）

## 5. 兼容性与迁移

### 5.1 向后兼容

- `Engine::new(spider)` 单 Spider 构造保留，等价于 `add_spider(spider)` + 单 Spider 共享队列
- `Spider` trait 新增方法都有默认实现，现有 Spider 实现无需改动
- `EngineBuilder::max_pages` 保留，作为引擎级兜底
- `Spider::matches` 默认实现遍历 `patterns()`，空 patterns 匹配所有（等价于旧行为）

### 5.2 测试策略

- 单 Spider 场景：行为不变，验证现有测试通过
- 多 Spider 场景：新增 E2E 测试，验证 URL 路由 + until 终止
- 缓存命中统计：新增单元测试，验证 `from_cache` 时 `pages` 不递增

## 6. 不做的事

- 不实现 `pages_by_tag` 分类计数（YAGNI，当前 `until(MaxPages(50))` 已满足需求）
- 不实现 `is_finished` 回调（引擎自己判断队列空 + in_flight）
- 不实现 `keepAlive` 模式（YAGNI）
- 不实现 Scrapy 的 `pagecount_since_last_item`（YAGNI）
- 不重构 `rule_engine` 的 per-spider 化（单独任务）

## 7. 风险

| 风险 | 缓解 |
|---|---|
| 共享队列锁竞争 | `Scheduler` 已用 `tokio::sync::Mutex`，异步不阻塞 runtime |
| `until` 每次构造 `StopContext` clone 开销 | 只读快照，usize/Duration 都是 Copy，开销可忽略 |
| `Arc<dyn StopCondition>` clone | `Arc::clone` 原子操作，开销极低 |
| 现有测试可能因架构改动失败 | 分步迁移，先加新接口再改内部实现 |
