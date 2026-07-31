# 爬虫框架对比研究与 wisp 架构评审

> **日期**：2026-07-23
> **范围**：仅研究与架构评审（不含实施）
> **方法论**：Web 调研 5 个主流爬虫框架 + 直接阅读 wisp 源码（~9.6k LOC）做架构评审 + 综合产出优化方案
> **基线 commit**：master `846e6b6`（已完成 code-review-2026-07-23 修复）

---

## 目录

1. [研究目的与范围](#1-研究目的与范围)
2. [5 框架对比研究](#2-5-框架对比研究)
3. [wisp 架构评审](#3-wisp-架构评审)
4. [对标优化方案](#4-对标优化方案)
5. [wisp 差异化定位建议](#5-wisp-差异化定位建议)
6. [附录：研究来源](#6-附录研究来源)

---

## 1. 研究目的与范围

### 1.1 目的

wisp 是一个 Rust 编写的爬虫框架（~9.6k LOC，已通过 206 lib 测试 + 12 integration 测试）。本研究旨在：

1. 调研 Scrapy / Crawlee / Colly / Scrapling / Spider-rs 五个主流爬虫框架的设计模式与最佳实践
2. 评审 wisp 当前架构的性能瓶颈、结构性低效与可扩展性限制
3. 对标一流框架产出分层优化方案，作为后续重构的输入

### 1.2 范围边界

**包含**：
- 框架特性对比表
- wisp 模块级架构评审（性能/结构/可扩展性/错误处理）
- 按优先级分层的优化建议（P0/P1/P2）

**不包含**（需用户审批后另起 spec）：
- 实际代码重构
- API 简化
- 文档更新
- benchmark 实测

### 1.3 评审覆盖的 wisp 模块

| 模块 | 文件 | LOC | 评审重点 |
|---|---|---|---|
| crawl/mod.rs | Spider trait / SpiderResponse / CrawlEvent | 507 | API 设计 |
| crawl/engine.rs | EngineContext / process_request | 814 | 流程拆分 |
| crawl/runner.rs | Engine / EngineBuilder / run_inner | 385 | 并发模型 |
| crawl/builder.rs | SpiderBuilder / ClosureSpider | 582 | callback 路由 |
| crawl/scheduling/scheduler.rs | Scheduler / DedupStrategy | 238 | 去重与队列 |
| crawl/runtime/autoscale.rs | AutoscaledPool | 148 | 自适应并发 |
| crawl/middleware/mod.rs | Middleware / ItemPipeline | 264 | 中间件链 |
| browser/pool.rs | BrowserPool | 369 | 浏览器池 |
| fetcher/mod.rs | Fetcher / FetchMode | 543 | 三模式分发 |

---

## 2. 5 框架对比研究

### 2.1 框架详细调研

#### Scrapy（Python）

- **仓库**：[github.com/scrapy/scrapy](https://github.com/scrapy/scrapy) ｜ ~63k stars
- **最新版本**：v2.17.0（2026-07-07，新增 HTTP/2 与 SOCKS 代理）
- **许可证**：BSD-3-Clause
- **核心特性**：Twisted reactor 异步引擎、CSS+XPath 链式选择器、Item Pipeline / Middlewares / Signals / Extensions 完整扩展体系、Feed Exports、AutoThrottle、`scrapy startproject` 脚手架
- **JS 渲染**：核心不内置，需 `scrapy-playwright` 扩展
- **反爬机制**：核心不内置，靠扩展（`scrapy-zyte-api`、`scrapy-fake-useragent`）
- **并发模型**：Twisted 单 reactor + 协程；GIL 限制多核
- **性能**：2C4G 服务器 ~300 req/s
- **优势**：生态最成熟、文档最全、企业首选、15+ 年稳定迭代
- **劣势**：Twisted 单 reactor CPU 密集解析阻塞、无原生 JS 渲染与反检测、GIL 限制多核

#### Crawlee（Node.js/TS + Python）

- **仓库**：[github.com/apify/crawlee](https://github.com/apify/crawlee) ｜ ~15k stars
- **最新版本**：3.17.0（TS） / 1.2.x（Python，2025-12）
- **许可证**：Apache-2.0
- **核心特性**：多 Crawler 统一 API（CheerioCrawler / PuppeteerCrawler / PlaywrightCrawler / BasicCrawler / StagehandCrawler）、持久化 RequestQueue、Result Storage（Dataset/KVS/Queue）、**反封锁系统开箱即用最完整**（got-scraping HTTP/2 + TLS 指纹、SessionPool、ProxyConfiguration、人机指纹）、BrowserPool、AutoscaledPool、`npx crawlee create` 脚手架
- **JS 渲染**：完整支持 Playwright + Puppeteer 双引擎
- **反爬机制**：开箱即用最完整
- **并发模型**：Node.js 事件循环 + AutoscaledPool 自适应
- **性能**：CheerioCrawler 4GB/1C ~500 页/分钟；PlaywrightCrawler 重型
- **优势**：反封锁能力开箱即用、多 Crawler 统一 API、Apify 云原生
- **劣势**：Node.js 单线程 CPU 密集受限、浏览器模式资源消耗大

#### Colly（Go）

- **仓库**：[github.com/gocolly/colly](https://github.com/gocolly/colly) ｜ ~22.9k stars
- **最新版本**：v2.2.0
- **许可证**：Apache-2.0
- **核心特性**：Collector + 事件驱动回调（OnRequest/OnResponse/OnHTML/OnXML/OnError/OnScraped）、goquery CSS 选择器、自动 Cookie/Referer/UA、域名白名单、深度控制、URL 过滤（regex/glob）、`c.Limit(LimitRule{Parallelism, Delay, DomainGlob})`、异步模式、**可插拔 storage 后端**（内存/Redis/BoltDB/SQLite）、分布式（Redis storage）、robots.txt 支持
- **JS 渲染**：核心不支持，需 chromedp / rod
- **反爬机制**：基础（UA、Cookie、代理、延迟）；无 TLS 指纹或 stealth
- **并发模型**：goroutine + channel；单核 1k+ req/s
- **性能**：单核 1k+ req/s；10,000 HTML 解析比 Scrapy 快 47%
- **优势**：极简 API、Go 原生高性能低消耗、goroutine 并发优秀、分布式友好
- **劣势**：无 JS 渲染、无内置反检测/stealth、维护节奏慢、文档薄弱

#### Scrapling（Python）

- **仓库**：[github.com/D4Vinci/Scrapling](https://github.com/D4Vinci/Scrapling) ｜ ~70k stars（增长极快）
- **最新版本**：v0.4.11（2026-07-13）
- **许可证**：BSD-3-Clause
- **核心特性**：**三层 Fetcher 渐进增强**（Fetcher 纯 HTTP+TLS 指纹 / DynamicFetcher Playwright / StealthyFetcher Camoufox）、**自适应元素追踪**（SQLite 存元素多维特征：标签/class/id/文本/属性/DOM 路径/父兄节点，相似度算法自动重定位改版后元素）、类 Scrapy Spider 框架（v0.4+，并发爬取 + 流式输出 + 断点续爬）、MCP Server（`pip install "scrapling[ai]"`）、IPython Shell + curl2fetcher + CLI
- **JS 渲染**：完整支持 Playwright + Camoufox（定制 Firefox）
- **反爬机制**：**开箱即用最强**——TLS 指纹、Canvas/WebGL/Audio/Navigator 指纹伪装、自动绕过 Cloudflare Turnstile / Datadome / Akamai / PerimeterX
- **并发模型**：asyncio + Spider 并发调度
- **性能**：比 BeautifulSoup 快 1775×、比 AutoScraper 快 5.1×
- **优势**：自适应解析（革命性降低维护成本）、反检测顶尖、性能卓越、三层 Fetcher 统一 API
- **劣势**：相对年轻（v0.4.x）、Camoufox 依赖较重

#### Spider-rs（Rust）

- **仓库**：[github.com/spider-rs/spider](https://github.com/spider-rs/spider) ｜ 2,205 commits，9 个 crate
- **最新版本**：v2.48.13（2026-03-31）
- **许可证**：MIT
- **核心特性**：Tokio 异步 + 零拷贝 HTML 解析、**实时流式输出**（Tokio broadcast channels，页面到达即推送）、**Smart 模式**（先 HTTP，检测到需要 JS 时透明升级 headless Chrome）、反检测（指纹模拟 + proxy hedging）、配置齐全（robots/sitemaps/per-path budgets/depth/glob/regex/cron）、**自适应并发**、per-domain 限速、HTTP/2 多路复用、Linux io_uring、去中心化模式（spider_worker 跨进程 IPC）、9 个 crate（spider/spider_cli/spider_worker/spider_agent/spider_mcp 等）、多语言绑定（Rust/CLI/Node/Python/MCP）、Spider Cloud 商业后端
- **JS 渲染**：支持，`features=["chrome"]` + `crawl_smart()`，CDP 控制 headless Chrome
- **反爬机制**：内置 stealth Chrome + 指纹 + proxy hedging；Spider Cloud 自动 anti-bot
- **并发模型**：Tokio async + io_uring + HTTP/2 多路复用
- **性能**：150,387 pages（espn.com）1 分钟完成；Spider Cloud 100k+ pages/second、p99 延迟 12ms；stealth benchmark 80 个反爬站点 85% 通过率
- **资源消耗**：极低（Rust 原生 + jemalloc + 字符串驻留）
- **优势**：Rust 性能顶尖、流式低延迟、Smart 模式智能切换、多语言绑定、商业化可持续
- **劣势**：社区规模小、文档以官网+examples 为主

### 2.2 对比总表

| 框架 | 语言 | 许可证 | Stars | 最新版本 | JS 渲染 | 反爬机制 | 并发模型 | 性能 | 维护活跃度 | 适合场景 |
|---|---|---|---|---|---|---|---|---|---|---|
| Scrapy | Python | BSD-3 | ~63k | v2.17.0 (2026-07) | ❌ 核心，需扩展 | ❌ 核心，需扩展 | Twisted reactor + 协程（GIL） | ~300 req/s (2C4G) | ⭐⭐⭐⭐⭐ 极活跃 | 大规模结构化爬取、企业平台 |
| Crawlee | TS/JS+Py | Apache-2.0 | ~15k | 3.17.0 | ✅ Playwright/Puppeteer | ✅ 开箱即用最完整 | Node 事件循环 + AutoscaledPool | ~500/min (1C4G) | ⭐⭐⭐⭐ 活跃 | 生产级动态站点、反爬对抗 |
| Colly | Go | Apache-2.0 | ~22.9k | v2.2.0 | ❌ 需 chromedp | ❌ 基础 | goroutine + channel | 1000+ (单核) | ⭐⭐⭐ 稳定 | 高并发静态页、Go 生态 |
| Scrapling | Python | BSD-3 | ~70k | v0.4.11 (2026-07) | ✅ Playwright + Camoufox | ✅ 顶尖 | asyncio + Spider | 比 BS4 快 1775× | ⭐⭐⭐⭐⭐ 极活跃 | 强反爬站点、自适应解析 |
| Spider-rs | Rust | MIT | 2.2k commits | v2.48.13 (2026-03) | ✅ Smart 模式 | ✅ stealth + hedging | Tokio + io_uring + HTTP/2 | 150k 页/min | ⭐⭐⭐⭐ 活跃 | 极致性能、流式、AI 数据管道 |

### 2.3 关键维度深度对比

#### 2.3.1 JavaScript 渲染能力

| 框架 | 机制 | 智能切换 | 备注 |
|---|---|---|---|
| Scrapy | scrapy-playwright（扩展） | ❌ | 需手动配置 |
| Crawlee | Playwright + Puppeteer | ❌ | 按 Crawler 类型选择 |
| Colly | ❌ 核心，需 chromedp | ❌ | Go 生态短板 |
| Scrapling | Playwright + Camoufox | ✅ 三层 Fetcher 渐进增强 | Camoufox 反检测最强 |
| Spider-rs | headless Chrome (CDP) | ✅ Smart 模式自动升级 | 只在需要时付 Chrome 税 |

#### 2.3.2 反爬机制完整度

| 能力 | Scrapy | Crawlee | Colly | Scrapling | Spider-rs |
|---|---|---|---|---|---|
| UA 轮换 | 扩展 | ✅ | ✅ | ✅ | ✅ |
| 代理池 | 扩展 | ✅ ProxyConfiguration | ✅ | ✅ ProxyRotator | ✅ + hedging |
| TLS 指纹 | 扩展 | ✅ got-scraping | ❌ | ✅ curl_cffi | ✅ |
| 浏览器指纹 | 扩展 | ✅ 人机指纹 | ❌ | ✅ Canvas/WebGL/Audio/Navigator 全伪装 | ✅ stealth Chrome |
| Session 管理 | 扩展 | ✅ SessionPool | ✅ Cookie | ✅ 持久化 Session | ✅ |
| 自动绕过 Cloudflare | ❌ | retryOnBlocked | ❌ | ✅ Turnstile 自动解决 | ✅ Spider Cloud Unblocker |
| 请求重试 | ✅ | ✅ maxRequestRetries | ✅ | ✅ | ✅ 自动重试 |

#### 2.3.3 性能与资源消耗

| 框架 | 吞吐量 | 内存占用 | CPU 利用 | 长跑适合度 |
|---|---|---|---|---|
| Scrapy | ~300 req/s (2C4G) | 中 | 单进程受 GIL，多核需多进程 | ⭐⭐⭐⭐ |
| Crawlee | ~8 p/s (Cheerio, 1C4G) | 中-高（浏览器 1GB+/实例） | Node 单线程 | ⭐⭐⭐ |
| Colly | 1000+ req/s (单核) | 低 | Go 原生多核 | ⭐⭐⭐⭐⭐ |
| Scrapling | 比 BS4 快 1775× | 极低（Fetcher 模式） | asyncio，GIL 限制 | ⭐⭐⭐⭐ |
| Spider-rs | 100k+ p/s（官方）；150k 页/min | 极低（Rust + jemalloc + 字符串驻留） | Tokio 多核 + io_uring | ⭐⭐⭐⭐⭐ |

#### 2.3.4 并发模型对比

| 框架 | 模型 | 多核利用 | 背压机制 |
|---|---|---|---|
| Scrapy | Twisted reactor + 协程 | ❌ 单进程单核（GIL），多核需 Scrapy-Redis | AutoThrottle |
| Crawlee | Node.js 事件循环 + AutoscaledPool | ❌ 单线程 | 有界队列 + 自适应并发 |
| Colly | goroutine + channel | ✅ Go 原生多核 | LimitRule + 有界队列 |
| Scrapling | asyncio + Spider 调度 | ❌ GIL 限制 | per-domain 节流 |
| Spider-rs | Tokio async + io_uring | ✅ Rust 原生多核 | Tokio broadcast + 自适应并发 |

---

## 3. wisp 架构评审

### 3.1 现有架构优点（无需改动）

对标 5 框架研究后，wisp 已有 4 个设计优于多数竞品：

1. **Engine 与 Spider 解耦**（[src/crawl/runner.rs:24-37](file:///home/weng/wisp/src/crawl/runner.rs)）
   - `Engine::infra().build()` 长期持有 HTTP client / SQLite 缓存 / RequestCache，`run(spider)` 多次复用
   - 比 Scrapy 的 Engine+Spider 耦合更优，比 Crawlee 的 Crawler 实例化更灵活

2. **StopCondition 可组合 trait**（`MaxPages.or(Timeout)`）
   - 比各框架硬编码参数更优雅，支持 `and` / `or` / `not` 组合

3. **SpiderBuilder 闭包式构建 + callback 路由**（[src/crawl/builder.rs:326-340](file:///home/weng/wisp/src/crawl/builder.rs)）
   - `on(label, handler)` 注册多 callback，对标 Scrapy callback 机制
   - 比 Scrapy 的 class 继承更轻量，比 Colly 的回调注册更结构化

4. **dev_mode + checkpoint + RequestCache 三层缓存**
   - dev_mode SQLite 快照便于开发回放
   - checkpoint 含 pending + seen 双集合恢复（[src/crawl/engine.rs:523-554](file:///home/weng/wisp/src/crawl/engine.rs)）
   - RequestCache 键含 HTTP method（避免 GET/POST 串味）

### 3.2 性能瓶颈（P0/P1）

#### A1. 全局 domain_sems 单锁 HashMap（P0）

[src/crawl/engine.rs:43](file:///home/weng/wisp/src/crawl/engine.rs) 与 [src/crawl/engine.rs:209-214](file:///home/weng/wisp/src/crawl/engine.rs)：

```rust
pub domain_sems: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>,
// ...
let sem = {
    let mut sems = ctx.domain_sems.lock().await;
    sems.entry(domain)
        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
        .clone()
};
```

**问题**：每请求都 `lock().await` 全局 HashMap 查找/插入，高并发下 push/pop 串行化。
**对标**：Crawlee 用 `ProxyConfiguration` 与 `SessionPool` 各自独立锁；spider-rs 用 sharded map。
**建议**：改用 `DashMap<String, Arc<Semaphore>>` 或 `arc-swap` + per-domain 独立 Mutex。

#### A2. status_codes 每页都锁（P1）

[src/crawl/engine.rs:480-483](file:///home/weng/wisp/src/crawl/engine.rs)：

```rust
async fn record_status(stats: &Arc<SpiderStats>, status: u16) {
    let mut m = stats.status_codes.lock().await;
    *m.entry(status).or_insert(0) += 1;
}
```

**问题**：每次响应都 `lock().await` 写状态码计数，主循环竞争点。
**建议**：用 `DashMap<u16, AtomicUsize>` 或每 worker 本地累积后批量合并。

#### A3. proxy_clients 同样全局锁（P1）

[src/crawl/engine.rs:45](file:///home/weng/wisp/src/crawl/engine.rs) 与 [src/crawl/engine.rs:621-633](file:///home/weng/wisp/src/crawl/engine.rs)：

```rust
pub proxy_clients: Arc<Mutex<HashMap<String, Arc<Client>>>>,
// ...
let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
    let mut cache = proxy_clients.lock().await;
    if !cache.contains_key(proxy) { /* build + insert */ }
    Some(cache.get(proxy).unwrap().clone())
}
```

**问题**：与 A1 同型——每代理请求都全局锁。
**建议**：同 A1，改用 `DashMap` 或在 `Engine` 启动时预建所有 proxy client。

#### A4. AutoscaledPool 已写但未集成（P0）

[src/crawl/runtime/autoscale.rs](file:///home/weng/wisp/src/crawl/runtime/autoscale.rs) 完整实现了自适应并发池（saturation > 0.9 扩容、< 0.7 缩容、错误率高缩容），但 [src/crawl/runner.rs](file:///home/weng/wisp/src/crawl/runner.rs) 的 `EngineBuilder` **没有 `.autoscale(min, max)` 入口**，`run_inner` 仍用固定 `buffer_unordered(max_concurrent)`（[src/crawl/runner.rs:312](file:///home/weng/wisp/src/crawl/runner.rs)）。

**对标**：Crawlee 的 AutoscaledPool 是核心卖点；spider-rs 也用自适应并发。
**建议**：在 `EngineBuilder` 增加 `.autoscale(min, max, config)`，主循环改为动态读取 `pool.current_concurrency()` 重建 stream。

#### A5. Scheduler 单 Mutex<BinaryHeap>（P1）

[src/crawl/scheduling/scheduler.rs:51-65](file:///home/weng/wisp/src/crawl/scheduling/scheduler.rs)：

```rust
struct SchedulerInner {
    heap: BinaryHeap<PrioritizedRequest>,
    seen_exact: HashSet<String>,
    seen_fp: HashSet<u64>,
    strategy: DedupStrategy,
    seq: u64,
}
pub struct Scheduler { inner: Arc<Mutex<SchedulerInner>> }
```

**问题**：push / pop / seen 检查共享同一 Mutex，高并发下成为串行点。
**对标**：spider-rs 用 lock-free 或 sharded scheduler；Colly 用 channel 解耦。
**建议**：分离 seen 集合（`DashSet` 或 `Arc<RwLock<HashSet>>`）与 heap（独立 Mutex）；或考虑 `crossbeam-queue`。

#### A6. apply_delay 同步阻塞 task（P2）

[src/crawl/engine.rs:485-500](file:///home/weng/wisp/src/crawl/engine.rs) 用 `tokio::time::sleep`，单 task 内阻塞，无法在 delay 期间切换工作。
**对标**：Crawlee 用 `tokio_ticker` 非忙等。
**建议**：低优先级，当前 `tokio::time::sleep` 已让出 runtime，问题不大。

#### A7. 每请求重复 url::Url::parse（P2）

[src/crawl/engine.rs:90](file:///home/weng/wisp/src/crawl/engine.rs) 域名过滤、[src/crawl/engine.rs:205](file:///home/weng/wisp/src/crawl/engine.rs) 信号量获取、`resolve_href` 内部都重复 parse URL。
**建议**：在 `SpiderRequest` 构造时缓存 `host` 与 `parsed_url`（`OnceCell<url::Url>`）。

### 3.3 结构性低效（P1）

#### B1. EngineContext 30+ 字段大结构体

[src/crawl/engine.rs:37-75](file:///home/weng/wisp/src/crawl/engine.rs) 单一 `EngineContext` 持有：共享资源（client/sched/robots_cache/...）+ per-Spider 配置（fetcher_config/fetch_mode/max_concurrent/...）+ 运行时状态（stats/items/control/...）混合。

**问题**：
- 每 spawn task 需 clone `Arc<EngineContext>`，Arc 内部数据量大，缓存友好性差
- 修改任一字段都需要审视全部调用点

**建议**：拆分为三层
- `EngineConfig`（只读，启动后不变）：client / fetcher_config / max_concurrent / obey_robots / ...
- `EngineShared`（跨 task 共享可变）：sched / robots_cache / domain_sems / proxy_clients / cache_store / ...
- `EngineState`（per-run 可变）：stats / items / control / abort_flag / ...

#### B2. process_request 200 行单函数（P1）

[src/crawl/engine.rs:80-263](file:///home/weng/wisp/src/crawl/engine.rs) `process_request` 处理：域名过滤 → 深度检查 → 控制状态 → 异步钩子 → 中间件 → 缓存 → robots → 信号量 → 延迟 → 重试 → handle，圈复杂度高。

**对标**：Crawlee 的 `BasicCrawler.runRequestHandler` 拆为 `handleRequestContext` + 多个独立 middleware。
**建议**：拆为独立 stage 函数，主流程用 `pipeline.run(req)` 链式调用。

#### B3. 双轨重试逻辑

[src/crawl/engine.rs:364-423](file:///home/weng/wisp/src/crawl/engine.rs) `fetch_dispatch` 内 attempt 重试 与 [src/crawl/engine.rs:400-405](file:///home/weng/wisp/src/crawl/engine.rs) 中间件 `run_error_middlewares` 返回 Retry 重试并行存在。

**问题**：重试策略分散，难以全局观察与调优。
**建议**：统一到 `RetryMiddleware`，`fetch_dispatch` 仅做单次 fetch。

#### B4. Auto 模式升级逻辑单维度（用户决定暂不动）

[src/crawl/engine.rs:429-463](file:///home/weng/wisp/src/crawl/engine.rs) `auto_upgrade_check` 只检测「选择器 0 匹配」升级 Dynamic，未考虑选择器匹配数异常、文本内容阈值、HTTP 状态码（403/429 升级 Stealth）等维度。

> **用户反馈（2026-07-23）**：先不做 Smart 模式，也不动 Auto 模式。本项仅作记录，不进入实施计划。

#### B5. SpiderRequest.meta 不持久化

[src/crawl/mod.rs:82-83](file:///home/weng/wisp/src/crawl/mod.rs)：

```rust
#[serde(skip)]
pub meta: Value,
```

**问题**：checkpoint 恢复后 meta 丢失，跨进程爬取无法传递上下文。
**对标**：Scrapling 用 SQLite 存元素多维特征；Scrapy 的 Request.meta 完整序列化。
**建议**：换用 `serde_json::Value` + 自定义 `serialize_meta` 函数（避免 bincode `deserialize_any` 问题），或换 postcard / msgpack 替代 bincode。

#### B6. Method 枚举与转换重复

[src/crawl/mod.rs:53](file:///home/weng/wisp/src/crawl/mod.rs) `Method` 仅 4 种（Get/Post/Put/Delete），缺 PATCH/HEAD/OPTIONS；[src/crawl/engine.rs:142-147](file:///home/weng/wisp/src/crawl/engine.rs) Method→&str 转换重复 4 处 match。
**建议**（已在 code-review-2026-07-23 progress.md #3 记录）：抽 `Method::as_str()` 方法；扩展枚举或允许 `Method::Other(String)`。

#### B7. 中间件每次 run 重排

[src/crawl/runner.rs:231-237](file:///home/weng/wisp/src/crawl/runner.rs)：

```rust
middleware_chain: {
    let mut chain = middleware::MiddlewareChain::new();
    chain.middlewares = spider.middlewares();
    chain.pipelines = spider.pipelines();
    chain.sort(); // 按 priority 排序
    Arc::new(chain)
}
```

**问题**：每次 `run(spider)` 都 clone + sort，对静态 Spider 配置是浪费。
**建议**：Spider 可缓存已排序 chain（`OnceCell<MiddlewareChain>`），或 Engine 缓存 `(spider name → chain)` 映射。

### 3.4 可扩展性限制（P1/P2）

#### C1. Storage 后端单一（P1）

[src/storage/](file:///home/weng/wisp/src/storage) 只支持 SQLite（`rusqlite`）。无法内存模式（测试开销大），后端不可插拔。

**对标**：Colly 支持 内存/Redis/BoltDB/SQLite；Crawlee 支持 本地/云存储切换。
**建议**：抽象 `StorageBackend` trait，提供 `InMemory` / `SQLite` 两实现（Redis 后端随分布式一起不考虑）。

#### C2. Scheduler 无分布式模式（用户决定不考虑）

[src/crawl/scheduling/scheduler.rs](file:///home/weng/wisp/src/crawl/scheduling/scheduler.rs) 单进程内 `BinaryHeap`，无跨节点任务分发。

**对标**：Colly 通过 Redis storage 实现跨节点去重；spider-rs 有 `spider_worker` 跨进程 IPC。
> **用户反馈（2026-07-23）**：分布式不考虑。本项仅作记录，不进入实施计划。

#### C3. 无 Smart 模式（对标 spider-rs）（用户决定暂不动）

wisp 的 `FetchMode::Auto`（[src/crawl/mod.rs](file:///home/weng/wisp/src/crawl/mod.rs)）只做「HTTP → 选择器 0 匹配 → Dynamic」单维度升级，未对标 spider-rs 的 Smart 模式（HTTP → blocked 检测 → Stealth → SelectorTracker 三阶段智能切换 + 嗅探规则缓存）。

> **用户反馈（2026-07-23）**：先不做 Smart 模式，也不动 Auto 模式。本项仅作记录，不进入实施计划。

#### C4. 反检测能力薄弱（P1）

wisp 现有：
- TLS 指纹模拟（wreq + Profile::Chrome136）
- Stealth 模式 CF 挑战解决（[src/stealth/challenge.rs](file:///home/weng/wisp/src/stealth/challenge.rs)）
- Turnstile 基础处理（[src/stealth/turnstile.rs](file:///home/weng/wisp/src/stealth/turnstile.rs)）

**缺失**（对标 Crawlee/Scrapling）：
- SessionPool（自动管理 Cookies + 旋转 Session）
- Canvas/WebGL/Audio/Navigator 浏览器指纹伪装
- retryOnBlocked 机制（统一 blocked 检测后重试策略）

**建议**：实现 `SessionPool` 中间件 + Cookie 池中间件，参考 Crawlee 设计。

#### C5. 流式背压 API 缺失（P2）

[src/crawl/runner.rs:89](file:///home/weng/wisp/src/crawl/runner.rs) `run_stream` 用 `mpsc::channel(128)` 固定缓冲，消费者慢时生产者阻塞，但无显式 backpressure 策略（如 drop / spill / sample）。

**对标**：spider-rs 用 Tokio broadcast channels，支持多消费者 + 容量溢出策略。
**建议**：暴露 `BackpressurePolicy` 枚举（Block / DropOldest / Spill）。

#### C6. MCP 模块未一等公民化（P2）

[src/mcp/](file:///home/weng/wisp/src/mcp) 模块存在但未在 `lib.rs` 顶层 re-export（[src/lib.rs:41](file:///home/weng/wisp/src/lib.rs) `pub mod mcp;` 但未 `pub use`），无完整 MCP server 入口。

**对标**：Scrapling 的 `pip install "scrapling[ai]"` MCP server 注册到 Claude Desktop / Cursor；spider-rs 有独立 `spider_mcp` crate。
**建议**：作为长期方向，对标 scrapling 实现完整 MCP server。

#### C7. 无 CLI 脚手架（用户决定不考虑）

无 `wisp new my-spider` 命令生成项目结构。

**对标**：Scrapy `startproject`、Crawlee `npx crawlee create`、Scrapling CLI 都有。
> **用户反馈（2026-07-23）**：CLI 脚手架不考虑。本项仅作记录，不进入实施计划。

#### C8. 去重策略单一（P2）

[src/crawl/scheduling/scheduler.rs:17-23](file:///home/weng/wisp/src/crawl/scheduling/scheduler.rs) 仅 Exact / Fingerprint 二选一。

**对标**：spider-rs 用字符串驻留 + Bloom filter；Scrapy 用 URL 归一化（去 fragment / 排序 query / lowercase host）。
**建议**：增加 `DedupStrategy::Normalized`（URL 归一化）+ `DedupStrategy::Bloom`（千万级 URL 场景）。

### 3.5 错误处理与可观察性（P2）

#### D1. WispError 粗粒度

[src/error.rs](file:///home/weng/wisp/src/error.rs) 主要是 `CdpError(String)` 等，无错误分类（网络/解析/反爬/超时/代理失败）。

**建议**：扩展为 enum，含 `Network` / `Parse` / `Blocked` / `Timeout` / `Proxy` / `Cdp` / `Storage` 变体，便于重试策略绑定。

#### D2. on_error 钩子信息少

[src/crawl/mod.rs:215](file:///home/weng/wisp/src/crawl/mod.rs) `async fn on_error(&self, _req: &SpiderRequest, _err: &str)` 只传 `&str`。

**建议**：改为 `on_error(&self, ctx: &ErrorContext)`，包含 req / response / attempt / error_type。

#### D3. tracing 散落无结构化字段

[src/crawl/engine.rs](file:///home/weng/wisp/src/crawl/engine.rs) 多处 `tracing::warn!` 散落，无统一字段（spider_name / url / attempt / error_type）。

**建议**：用 `tracing::Instrument` + span 统一上下文字段；或引入 `sentry` / `opentelemetry` 集成。

#### D4. CrawlStats 无时间序列指标

[src/crawl/mod.rs:249-263](file:///home/weng/wisp/src/crawl/mod.rs) `CrawlStats` 是累计值，无 p50/p99 响应时间、QPS 曲线、错误率时序。

**建议**：引入 `hdrhistogram` crate 记录响应时间分布，提供 `stats.percentile(99)` API。

---

## 4. 对标优化方案

按优先级分层，每项含「问题/对标/建议」三段式。

### 4.1 P0 高优先级（性能瓶颈 + 已写未集成）

| ID | 问题 | 对标 | 建议 |
|---|---|---|---|
| **P0-1** | AutoscaledPool 已写未集成 | Crawlee AutoscaledPool | `EngineBuilder.autoscale(min, max, config)`，主循环改动态并发 |
| **P0-2** | domain_sems 全局单锁 HashMap | Crawlee 独立 SessionPool 锁 | 改 `DashMap<String, Arc<Semaphore>>` |
| **P0-3** | EngineContext 30+ 字段混合 | Crawlee Context 分层 | 拆 `EngineConfig(只读)` + `EngineShared(可变共享)` + `EngineState(per-run)` |
| **P0-4** | process_request 200 行单函数 | Crawlee middleware 拆分 | 拆为独立 stage 函数，主流程链式调用 |

### 4.2 P1 中优先级（结构性 + 可扩展性）

| ID | 问题 | 对标 | 建议 |
|---|---|---|---|
| **P1-1** | status_codes / proxy_clients 每请求锁 | spider-rs atomic | `DashMap<u16, AtomicUsize>` + 预建 proxy client |
| **P1-2** | Scheduler 单 Mutex<BinaryHeap> | spider-rs sharded | 分离 seen（`DashSet`）与 heap（独立 Mutex） |
| **P1-3** | 反检测能力薄弱 | Crawlee/Scrapling | SessionPool + Cookie 池中间件 |
| **P1-4** | Storage 后端单一 | Colly 多后端 | `StorageBackend` trait + InMemory/SQLite 实现（Redis 随分布式不考虑） |
| **P1-5** | Method 枚举与转换重复 | （code-review #3） | `Method::as_str()` + 扩展枚举 |
| **P1-6** | 双轨重试逻辑 | Crawlee RetryMiddleware | 统一到中间件，`fetch_dispatch` 仅单次 fetch |
| **P1-7** | SpiderRequest.meta 不持久化 | Scrapy Request.meta | 自定义 serde 或换 postcard/msgpack |

### 4.3 P2 长期方向（可扩展性 + 工具链）

| ID | 问题 | 对标 | 建议 |
|---|---|---|---|
| **P2-1** | 流式无背压策略 | spider-rs broadcast | `BackpressurePolicy` 枚举（Block/DropOldest/Spill） |
| **P2-2** | MCP 模块未一等公民 | scrapling/spider-rs MCP | 完整 MCP server，注册 Claude/Cursor |
| **P2-3** | 去重策略单一 | spider-rs Bloom / Scrapy 归一化 | `Normalized` + `Bloom` 策略 |
| **P2-4** | WispError 粗粒度 | Crawlee 错误分类 | enum 变体 Network/Parse/Blocked/Timeout/Proxy |
| **P2-5** | tracing 无结构化字段 | opentelemetry | span 统一 spider_name/url/attempt/error_type |
| **P2-6** | CrawlStats 无时序指标 | hdrhistogram | p50/p99/QPS 曲线 |
| **P2-7** | 每请求重复 url::Url::parse | （内部优化） | `SpiderRequest.host: OnceCell<String>` |
| **P2-8** | 中间件每次 run 重排 | （内部优化） | `Spider.cached_chain: OnceCell<MiddlewareChain>` |

---

## 5. wisp 差异化定位建议

基于对比研究，wisp 在 Rust 爬虫生态中应形成 **"高性能 + 低维护 + AI 友好"** 的差异化定位：

### 5.1 核心定位

```
Rust 高性能爬虫框架，对标 spider-rs 但偏框架化可嵌入（vs spider-rs 偏云服务商业化）：
- 性能：Tokio + io_uring + HTTP/2（学习 spider-rs）
- 低维护：自适应元素追踪（学习 Scrapling，深化现有 SelectorTracker）
- AI 友好：MCP server 一等公民（学习 scrapling/spider-rs）
```

### 5.2 短期补齐（P0+P1）

1. **集成 AutoscaledPool**——对标 Crawlee 自适应并发
2. **拆分 EngineContext**——降低耦合，提升缓存友好性
3. **SessionPool + Cookie 池**——对标 Crawlee 反封锁
4. **Storage trait 抽象**——对标 Colly 多后端（InMemory/SQLite）

### 5.3 长期差异化（P2）

1. **自适应元素追踪**（深化现有 SelectorTracker）——对标 Scrapling 杀手级特性
2. **MCP server 完整化**——对标 scrapling AI 集成

### 5.4 不建议跟进

- **不**做云服务商业化（spider-rs 已占位，wisp 定位框架）
- **不**做 Camoufox 级深度浏览器指纹（依赖重，与 Rust 生态不符；改用 CDP stealth patches）
- **不**做多语言绑定（spider-rs 已覆盖；wisp 专注 Rust + MCP）
- **不**做 Smart 模式与 Auto 模式改造（用户 2026-07-23 决定暂不动）
- **不**做分布式 Scheduler 与 CLI 脚手架（用户 2026-07-23 决定不考虑）

---

## 6. 附录：研究来源

### 6.1 框架官方资源

- [Scrapy 官网](https://scrapy.org/) ｜ [Scrapy GitHub](https://github.com/scrapy/scrapy) ｜ [Scrapy 文档](https://docs.scrapy.org/en/latest/news.html)
- [Crawlee 官网](https://crawlee.dev/) ｜ [Crawlee GitHub](https://github.com/apify/crawlee)
- [Colly 官网](http://go-colly.org/) ｜ [Colly GitHub](https://github.com/gocolly/colly) ｜ [Colly pkg.go.dev](https://pkg.go.dev/github.com/gocolly/colly/v2)
- [Scrapling GitHub](https://github.com/D4Vinci/Scrapling) ｜ [Scrapling 文档](https://scrapling.readthedocs.io/) ｜ [Scrapling PyPI](https://pypi.org/project/scrapling/)
- [Spider-rs GitHub](https://github.com/spider-rs/spider) ｜ [Spider Cloud 官网](https://spider.cloud/) ｜ [spider crate](https://crates.io/crates/spider)

### 6.2 Benchmark 与对比文章

- [Spider Browser Stealth Benchmark](https://spider.cloud/blog/spider-browser-stealth-benchmark)
- [Top 5 Data Collection Platforms 2026](https://spider.cloud/blog/top-5-data-collection-platforms)
- [Scrapling MCP Scorecard 报道](https://mcp-scorecard.ai/blog/20260301-scrapling-takes-the-crown/)
- [Scrapling 完全指南（The Web Scraping Club）](https://substack.thewebscraping.club/p/scrapling-hands-on-guide)
- [Scrapy 迁移 Crawlo 案例分析](https://cloud.tencent.com/developer/article/2710540)
- [Colly 性能基准测试](https://blog.csdn.net/gitblog_00586/article/details/154680406)
- [全球爬虫框架市场增长报告 2023-2025](https://juejin.cn/post/7625454584483905570)

### 6.3 wisp 内部参考

- [CLAUDE.md](file:///home/weng/wisp/CLAUDE.md) — 项目核心概念文档
- [.superpowers/sdd/progress.md](file:///home/weng/wisp/.superpowers/sdd/progress.md) — code-review-2026-07-23 修复进度（含 8 个 Minor 待跟进项）
- master commit `846e6b6` — 评审基线

### 6.4 调研说明

- 数据截至 2026-07-23，GitHub stars 等动态数据可能随时间变化
- 性能指标多来自官方 benchmark，实际表现受目标站点、网络环境、硬件配置影响
- Scrapling 的 stars 增长极快（2026-04 的 36k → 2026-07 的 70k+），使用时以最新数据为准
- wisp 架构评审基于直接阅读源码（~9.6k LOC），未做 runtime profiling

---

## 下一步

本 spec 为**研究与评审产出**，未含代码改动。如需进入实施阶段，建议：

1. 用户评审本文档，确认 P0/P1 优先级与范围
2. 对每个 P0/P1 项另起 spec（如 `2026-07-24-autoscale-integration-design.md`）
3. 用 writing-plans skill 生成实施 plan
4. 按 plan 分批实施，每批完成后回归测试

待用户确认是否进入实施，以及优先实施哪几项。
