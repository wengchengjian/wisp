# Tracing 性能埋点 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Engine 内部 6 个关键函数加 `#[instrument]` span，benchmark 里用自定义 TimingLayer 聚合各阶段 wall clock duration，打印百分比拆解。

**Architecture:** 纯 tracing span 方案——框架代码只加 `#[instrument]` 属性（零侵入、无 subscriber 时零开销），benchmark 里用 `TimingLayer`（实现 `tracing_subscriber::Layer`）通过 `on_new_span`/`on_close` 聚合 span duration。global subscriber 解决 tokio spawn worker 线程 span 丢失问题。

**Tech Stack:** Rust, tracing 0.1, tracing-subscriber 0.3（已在 dependencies）, criterion 0.5

## Global Constraints

- 不加 feature gate（tracing span 无 subscriber 时已零开销）
- 不改函数签名、不改函数体逻辑，只加 `#[instrument]` 属性
- `ctx`/`req`/`resp`/`crawl_ctx`/`self` 参数全部 `skip`（含大对象/引用，不需 Display）
- TimingLayer 放 `benches/` 目录，不污染框架代码
- global subscriber 用 `OnceLock` 保证只设一次
- 命名遵循 snake_case，提交信息用中文

---

## File Structure

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `src/crawl/engine.rs` | 4 个函数加 `#[instrument]` | Modify |
| `src/crawl/middleware/mod.rs` | 2 个函数加 `#[instrument]` | Modify |
| `benches/timing_layer.rs` | TimingLayer 实现（Layer trait + 聚合 stats + print_summary） | Create |
| `benches/bench.rs` | 集成 TimingLayer 到 engine_concurrent_fetch | Modify |
| `Cargo.toml` | 无改动（tracing + tracing-subscriber 已在依赖） | — |

---

### Task 1: engine.rs 4 个函数加 `#[instrument]`

**Files:**
- Modify: `src/crawl/engine.rs`（process_request / process_response / acquire_and_fetch / fetch_dispatch）

**Interfaces:**
- Consumes: `tracing::instrument`（已在 dependencies）
- Produces: 4 个函数带 span，无 subscriber 时 no-op

- [ ] **Step 1: 确认 engine.rs 顶部已 import tracing**

读 `src/crawl/engine.rs:12-20`，确认有 `use tracing::...` 或 `tracing` crate 可用。若无 `instrument` 宏，用全路径 `#[tracing::instrument]`。

- [ ] **Step 2: 为 process_request 加 #[instrument]**

在 `src/crawl/engine.rs` 找到 `pub(crate) async fn process_request(ctx: &EngineContext, req: Request) -> Option<Response>`，在上方加属性：

```rust
#[tracing::instrument(skip(ctx), fields(url = %req.url))]
pub(crate) async fn process_request(ctx: &EngineContext, req: Request) -> Option<Response> {
```

- [ ] **Step 3: 为 process_response 加 #[instrument]**

在 `src/crawl/engine.rs` 找到 `pub(crate) async fn process_response(ctx: &EngineContext, resp: Response)`，在上方加属性：

```rust
#[tracing::instrument(skip(ctx, resp), fields(status = resp.status))]
pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
```

注意：`resp.status` 是 `u16`（Copy 类型），在 span 创建时拷贝，不影响后续 resp 的 move。

- [ ] **Step 4: 为 acquire_and_fetch 加 #[instrument]**

在 `src/crawl/engine.rs` 找到 `async fn acquire_and_fetch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>)`，在上方加属性：

```rust
#[tracing::instrument(skip(ctx, req))]
async fn acquire_and_fetch(
    ctx: &EngineContext,
    req: &Request,
) -> (Option<Response>, Option<String>) {
```

注意：domain 在函数内部算，不加 field（TimingLayer 按 span name 聚合，不需 field）。

- [ ] **Step 5: 为 fetch_dispatch 加 #[instrument]**

在 `src/crawl/engine.rs` 找到 `async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>)`，在上方加属性：

```rust
#[tracing::instrument(skip(ctx, req))]
async fn fetch_dispatch(
    ctx: &EngineContext,
    req: &Request,
) -> (Option<Response>, Option<String>) {
```

- [ ] **Step 6: 编译验证**

Run: `cargo build 2>&1 | tail -5`
Expected: 0 error, 0 warning

- [ ] **Step 7: 运行测试确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 201 passed, 0 failed

- [ ] **Step 8: 提交**

```bash
git add src/crawl/engine.rs
git commit -m "engine: 为 4 个核心函数加 tracing #[instrument] span"
```

---

### Task 2: middleware/mod.rs 2 个函数加 `#[instrument]`

**Files:**
- Modify: `src/crawl/middleware/mod.rs`（run_request_middlewares / run_response_middlewares）

**Interfaces:**
- Consumes: `tracing::instrument`
- Produces: 2 个方法带 span

- [ ] **Step 1: 为 run_request_middlewares 加 #[instrument]**

在 `src/crawl/middleware/mod.rs` 找到 `pub(crate) async fn run_request_middlewares(&self, req: &mut Request, ctx: &CrawlContext) -> MwAction`，在上方加属性：

```rust
#[tracing::instrument(skip(self, req, ctx))]
pub(crate) async fn run_request_middlewares(
    &self,
    req: &mut Request,
    ctx: &CrawlContext,
) -> MwAction {
```

- [ ] **Step 2: 为 run_response_middlewares 加 #[instrument]**

在 `src/crawl/middleware/mod.rs` 找到 `pub(crate) async fn run_response_middlewares(&self, resp: &mut Response, ctx: &CrawlContext) -> MwAction`，在上方加属性：

```rust
#[tracing::instrument(skip(self, resp, ctx))]
pub(crate) async fn run_response_middlewares(
    &self,
    resp: &mut Response,
    ctx: &CrawlContext,
) -> MwAction {
```

- [ ] **Step 3: 编译验证**

Run: `cargo build 2>&1 | tail -5`
Expected: 0 error, 0 warning

- [ ] **Step 4: 运行测试确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 201 passed, 0 failed

- [ ] **Step 5: 提交**

```bash
git add src/crawl/middleware/mod.rs
git commit -m "middleware: 为请求/响应中间件链加 tracing #[instrument] span"
```

---

### Task 3: 实现 TimingLayer

**Files:**
- Create: `benches/timing_layer.rs`

**Interfaces:**
- Consumes: `tracing::Subscriber`, `tracing_subscriber::Layer`, `tracing_subscriber::registry::LookupSpan`
- Produces: `TimingLayer` struct（Clone）, `TimingLayer::new()`, `TimingLayer::reset()`, `TimingLayer::print_summary()`

- [ ] **Step 1: 创建 benches/timing_layer.rs**

写入完整实现：

```rust
//! Benchmark 专用：聚合 tracing span 的 wall clock duration，打印各阶段耗时百分比。
//!
//! 用 on_new_span（创建时记时间）而非 on_enter，因为 async span 可能多次
//! enter/exit（每次 poll），但创建到关闭的 wall clock = 该阶段真实耗时（含 I/O 等待）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::span::Id;
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Clone)]
pub struct TimingLayer {
    inner: Arc<Inner>,
}

struct Inner {
    /// span_id → 创建时间
    create_times: Mutex<HashMap<Id, Instant>>,
    /// span name → (总耗时, 调用次数)
    stats: Mutex<HashMap<String, (Duration, usize)>>,
}

impl TimingLayer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                create_times: Mutex::new(HashMap::new()),
                stats: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// 清空统计（每个 benchmark 级别前重置）
    pub fn reset(&self) {
        self.inner.create_times.lock().unwrap().clear();
        self.inner.stats.lock().unwrap().clear();
    }

    /// 按 total duration 降序打印各阶段耗时 + 百分比 + 调用次数
    pub fn print_summary(&self) {
        let stats = self.inner.stats.lock().unwrap();
        if stats.is_empty() {
            println!("  (no span data — subscriber not registered?)");
            return;
        }
        let mut entries: Vec<_> = stats.iter().collect();
        entries.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        let total = entries
            .iter()
            .find(|(name, _)| **name == "process_request")
            .map(|(_, (dur, _))| *dur)
            .unwrap_or_else(|| entries.iter().map(|(_, (d, _))| *d).max().unwrap_or_default());
        for (name, (dur, count)) in entries {
            let pct = if total.as_nanos() > 0 {
                dur.as_secs_f64() / total.as_secs_f64() * 100.0
            } else {
                0.0
            };
            println!(
                "  {:30} {:10.3} ms ({:5.1}%)  {} calls",
                name,
                dur.as_secs_f64() * 1000.0,
                pct,
                count
            );
        }
    }
}

impl<S> Layer<S> for TimingLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        _attrs: &tracing::span::Attributes<'_>,
        id: &Id,
        _ctx: Context<'_, S>,
    ) {
        self.inner
            .create_times
            .lock()
            .unwrap()
            .insert(id.clone(), Instant::now());
    }

    fn on_close(&self, id: &Id, ctx: Context<'_, S>) {
        let created = self.inner.create_times.lock().unwrap().remove(id);
        if let Some(created) = created {
            let dur = created.elapsed();
            let name = ctx
                .span_scope(id)
                .map(|s| s.name().to_string())
                .unwrap_or_default();
            let mut stats = self.inner.stats.lock().unwrap();
            let entry = stats.entry(name).or_insert((Duration::ZERO, 0));
            entry.0 += dur;
            entry.1 += 1;
        }
    }
}
```

- [ ] **Step 2: 在 benches/bench.rs 顶部声明 mod**

读 `benches/bench.rs` 第 1-10 行，在现有 `mod` 声明后加：

```rust
mod timing_layer;
```

- [ ] **Step 3: 编译验证**

Run: `cargo bench --bench bench --no-run 2>&1 | tail -10`
Expected: 0 error, 0 warning

- [ ] **Step 4: 提交**

```bash
git add benches/timing_layer.rs benches/bench.rs
git commit -m "bench: 添加 TimingLayer 聚合 tracing span 耗时"
```

---

### Task 4: 集成 TimingLayer 到 benchmark + 验证

**Files:**
- Modify: `benches/bench.rs`（engine_concurrent_fetch 函数 + 顶部 OnceLock）

**Interfaces:**
- Consumes: `TimingLayer`（Task 3）, `tracing_subscriber::registry`
- Produces: benchmark 输出各阶段百分比

- [ ] **Step 1: 在 bench.rs 顶部加 OnceLock 和 timing() 函数**

读 `benches/bench.rs` 第 1-15 行（imports 区）。在 imports 后加：

```rust
use std::sync::OnceLock;
use timing_layer::TimingLayer;

static TIMING: OnceLock<TimingLayer> = OnceLock::new();

/// 获取全局 TimingLayer（注册 global subscriber，只设一次）。
/// process_request 通过 tokio::spawn 在 worker 线程执行，
/// thread-local subscriber 抓不到，必须用 global。
fn timing() -> &'static TimingLayer {
    TIMING.get_or_init(|| {
        let layer = TimingLayer::new();
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(layer.clone()),
        );
        layer
    })
}
```

- [ ] **Step 2: 修改 bench_engine_concurrent_fetch 集成 TimingLayer**

读 `benches/bench.rs` 找到 `fn bench_engine_concurrent_fetch`。在 `for &concurrent in &[1usize, 4, 16]` 循环体内，`group.bench_with_input` 之前加 `timing.reset()`，之后加 `timing.print_summary()`：

```rust
fn bench_engine_concurrent_fetch(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let base = rt.block_on(spawn_html_server(BENCH_HTML));
    let urls: Vec<String> = (0..50).map(|i| format!("{}/p{}", base, i)).collect();

    let timing = timing();
    let mut group = c.benchmark_group("engine_concurrent_fetch");
    group.sample_size(20);
    for &concurrent in &[1usize, 4, 16] {
        let engine = Engine::infra()
            .max_concurrent(concurrent)
            .max_pages(50)
            .build()
            .unwrap();
        timing.reset();
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrent),
            &concurrent,
            |b, _| {
                b.iter(|| {
                    rt.block_on(async {
                        let spider = BenchSpider { urls: urls.clone() };
                        engine.run(spider).await.unwrap()
                    })
                })
            },
        );
        println!("engine_concurrent_fetch/{} - Stage Timing:", concurrent);
        timing.print_summary();
    }
    group.finish();
}
```

- [ ] **Step 3: 编译验证**

Run: `cargo bench --bench bench --no-run 2>&1 | tail -10`
Expected: 0 error, 0 warning

- [ ] **Step 4: 运行 benchmark 验证输出**

Run: `cargo bench --bench bench -- engine_concurrent_fetch -- --quick 2>&1 | tail -30`
Expected: 看到各阶段耗时百分比输出，类似：
```
engine_concurrent_fetch/1 - Stage Timing:
  process_request                   X.XXX ms (100.0%)  50 calls
  acquire_and_fetch                 X.XXX ms (XX.X%)   50 calls
  process_response                  X.XXX ms (XX.X%)   50 calls
  run_request_middlewares           X.XXX ms (XX.X%)   50 calls
  fetch_dispatch                    X.XXX ms (XX.X%)   50 calls
  run_response_middlewares          X.XXX ms (XX.X%)   50 calls
```

- [ ] **Step 5: 验证 parser benchmark 不受影响（零开销）**

Run: `cargo bench --bench bench -- parse -- --quick 2>&1 | tail -10`
Expected: 正常输出 parse/10KB 等结果，无 span 数据输出（parser benchmark 未集成 TimingLayer）

- [ ] **Step 6: 运行全量测试确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 201 passed, 0 failed

- [ ] **Step 7: 提交**

```bash
git add benches/bench.rs
git commit -m "bench: engine_concurrent_fetch 集成 TimingLayer 输出各阶段耗时"
```

---

## Self-Review

**1. Spec coverage:**
- 6 个函数加 #[instrument] → Task 1 (4个) + Task 2 (2个) ✓
- TimingLayer 实现 → Task 3 ✓
- benchmark 集成 global subscriber → Task 4 ✓
- wall clock duration（on_new_span/on_close）→ Task 3 实现 ✓
- OnceLock global subscriber → Task 4 Step 1 ✓
- reset + print_summary → Task 3 实现 ✓

**2. Placeholder scan:** 无 TBD/TODO，所有步骤有完整代码 ✓

**3. Type consistency:**
- `TimingLayer::new()` / `reset()` / `print_summary()` 在 Task 3 定义，Task 4 使用 ✓
- `timing()` 返回 `&'static TimingLayer` 在 Task 4 Step 1 定义，Step 2 使用 ✓
- `#[instrument(skip(...))]` 签名一致 ✓

**4. 已知限制（设计决策，非 bug）：**
- acquire_and_fetch 不加 domain field（内部变量，#[instrument] fields 只能用参数）
- fetch_dispatch 不加 mode field（避免依赖 EngineConfig.fetch_mode 字段是否存在）
- TimingLayer 按 span name 聚合，不区分 span 层级（足够 benchmark 用途）
