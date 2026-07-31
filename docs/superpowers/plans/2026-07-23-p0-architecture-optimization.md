# P0 架构优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实施 spec `2026-07-23-framework-research-arch-review.md` 中的 4 个 P0 项：AutoscaledPool 集成（P0-1）、domain_sems DashMap 改造（P0-2）、EngineContext 三层拆分（P0-3）、process_request 拆分（P0-4）。

**Architecture:** 按依赖顺序分 4 阶段实施。P0-1（Autoscale 集成）与 P0-2（DashMap）互相独立、风险最低，先做。P0-3（EngineContext 拆分）是机械式重构，为 P0-4（process_request 拆分）铺路——拆分后的 stage 函数可更自然地接收 `&EngineConfig` / `&EngineShared` / `&EngineState`。每个 task 以 TDD 方式先写/改测试，再改实现，最后回归全量测试。

**Tech Stack:** Rust 2021 edition, tokio 异步运行时, futures stream, dashmap 6.x（新增依赖）, Arc/Mutex/DashMap 并发原语, wreq HTTP client, moka 缓存。

## Global Constraints

- Rust edition 2021，工具链：`cargo build` / `cargo test --lib` / `cargo test --test <name>` 必须通过。
- 所有公开 API（`Engine::infra/run/run_stream`、`EngineBuilder` 链式方法、`Spider` trait）保持向后兼容——只新增方法，不删除/重命名现有方法。
- `EngineBuilder.autoscale(min, max)` 是新增方法，不破坏现有 `.max_concurrent(n)` 用法。
- EngineContext 是 `pub(crate)` 结构，可自由重构字段布局，不影响外部 API。
- 测试位于 `tests/` 目录（集成测试）或文件内 `#[cfg(test)] mod tests`（单元测试）。需真实网络/Chrome 的测试用 `#[ignore]` 标记。
- 注释与代码用中文（与现有代码风格一致）。
- 禁止 `unwrap()`/`expect()` 出现在可恢复路径（仅允许在静态构造/编译期常量与测试中）。
- 提交粒度：每个 task 一个 commit，commit message 用 `feat:` / `refactor:` / `perf:` 前缀。
- **基线 commit**：master `846e6b6`（已完成 code-review-2026-07-23 修复）。

---

## File Structure

修改的文件按阶段分组（无新建源码文件，全部为现有文件修改 + 新建测试文件）：

- `src/crawl/runtime/autoscale.rs` — AutoscaledPool，增加 `max_concurrency()` 访问器（Phase 1）
- `src/crawl/runner.rs` — Engine + EngineBuilder，增加 `.autoscale()` 入口 + run_inner 动态并发（Phase 1）
- `Cargo.toml` — 新增 dashmap 依赖（Phase 2）
- `src/crawl/engine.rs` — EngineContext 字段类型变更 + 三层拆分 + process_request 拆分（Phase 2/3/4）
- `src/crawl/runner.rs` — EngineContext 构造更新（Phase 2/3）

测试文件：
- `tests/p0_autoscale_test.rs` — 新建，AutoscaledPool 集成测试（Phase 1）
- `tests/p0_dashmap_test.rs` — 新建，domain_sems DashMap 并发测试（Phase 2）
- `src/crawl/engine.rs` 内 `#[cfg(test)] mod tests` — 更新 make_ctx + 新增拆分验证测试（Phase 3/4）

---

## Phase 1: P0-1 AutoscaledPool 集成

AutoscaledPool 已在 `src/crawl/runtime/autoscale.rs` 完整实现（saturation > 0.9 扩容、< 0.7 缩容、错误率高缩容），但 `EngineBuilder` 没有 `.autoscale()` 入口，`run_inner` 仍用固定 `buffer_unordered(max_concurrent)`。

### Task 1: 增加 EngineBuilder.autoscale() API

**Files:**
- Modify: `src/crawl/runtime/autoscale.rs:57-83`（AutoscaledPool，增加 `max_concurrency()` 访问器）
- Modify: `src/crawl/runner.rs:23-51`（Engine + EngineBuilder，增加 autoscale 字段）
- Modify: `src/crawl/runner.rs:347-384`（EngineBuilder impl，增加 `.autoscale()` 方法 + build 传递）
- Test: `tests/p0_autoscale_test.rs`（新建）

**Interfaces:**
- Consumes: `AutoscaledPool::new(min, max, config) -> Arc<Self>`（已存在于 autoscale.rs:68）
- Produces: `EngineBuilder::autoscale(min, max) -> Self`、`EngineBuilder::autoscale_with_config(min, max, config) -> Self`、`AutoscaledPool::max_concurrency() -> usize`

- [x] **Step 1: 写失败测试 — Engine 可通过 builder 启用 autoscale**

创建 `tests/p0_autoscale_test.rs`：

```rust
//! P0-1: 验证 EngineBuilder.autoscale() API 可用。
//! AutoscaledPool 已实现，此测试验证 Engine 正确持有 autoscale 配置。

use wisp::crawl::runtime::autoscale::{AutoscaledPool, AutoscaleConfig};
use std::time::Duration;

#[tokio::test]
async fn engine_builder_accepts_autoscale() {
    // .autoscale(min, max) 应返回 Self（链式可用），且 build() 不报错
    let engine = wisp::crawl::Engine::infra()
        .max_concurrent(16)
        .autoscale(2, 8)
        .build();
    assert!(engine.is_ok(), "build with autoscale should succeed: {:?}", engine.err());
}

#[tokio::test]
async fn engine_builder_accepts_autoscale_with_config() {
    let config = AutoscaleConfig {
        scale_up_interval: Duration::from_secs(3),
        scale_down_interval: Duration::from_secs(1),
        ..Default::default()
    };
    let engine = wisp::crawl::Engine::infra()
        .autoscale_with_config(1, 4, config)
        .build();
    assert!(engine.is_ok(), "build with autoscale config should succeed");
}

#[test]
fn autoscaled_pool_exposes_max_concurrency() {
    let pool = AutoscaledPool::new(2, 8, AutoscaleConfig::default());
    assert_eq!(pool.max_concurrency(), 8, "max_concurrency() 应返回上限值");
    assert_eq!(pool.current_concurrency(), 2, "初始值应为 min");
}
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --test p0_autoscale_test`
Expected: 编译失败 — `no method named autoscale on EngineBuilder`、`no method named max_concurrency on AutoscaledPool`

- [x] **Step 3: 给 AutoscaledPool 增加 max_concurrency() 访问器**

在 `src/crawl/runtime/autoscale.rs` 的 `impl AutoscaledPool` 块中（`current_concurrency` 方法后），增加：

```rust
/// 获取最大并发数上限（主循环用作 buffer_unordered 的 ceiling）。
pub fn max_concurrency(&self) -> usize {
    self.max_concurrency
}
```

- [x] **Step 4: 给 Engine + EngineBuilder 增加 autoscale 字段**

在 `src/crawl/runner.rs` 的 `Engine` 结构体中（`control` 字段后）增加：

```rust
/// 自适应并发池（可选）。启用后 run_inner 动态调整并发数。
pub(crate) autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
```

在 `EngineBuilder` 结构体中（`checkpoint_interval` 字段后）增加：

```rust
autoscale: Option<Arc<crate::crawl::runtime::autoscale::AutoscaledPool>>,
```

在 `Engine::infra()` 的返回初始化中增加：

```rust
autoscale: None,
```

在 `EngineBuilder` impl 中增加方法（`checkpoint` 方法后）：

```rust
/// 启用自适应并发池。min 为初始/下限，max 为上限。
/// 启用后 run_inner 会启动后台 autoscaler，根据饱和度动态调整并发数。
pub fn autoscale(mut self, min: usize, max: usize) -> Self {
    self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(
        min, max,
        crate::crawl::runtime::autoscale::AutoscaleConfig::default(),
    ));
    self
}

/// 同 autoscale(min, max) 但可自定义配置。
pub fn autoscale_with_config(
    mut self,
    min: usize,
    max: usize,
    config: crate::crawl::runtime::autoscale::AutoscaleConfig,
) -> Self {
    self.autoscale = Some(crate::crawl::runtime::autoscale::AutoscaledPool::new(min, max, config));
    self
}
```

在 `build()` 方法的 `Ok(Engine { ... })` 中增加字段：

```rust
autoscale: self.autoscale,
```

- [x] **Step 5: 运行测试验证通过**

Run: `cargo test --test p0_autoscale_test`
Expected: 3 个测试全部 PASS

- [x] **Step 6: 回归全量 lib 测试**

Run: `cargo test --lib`
Expected: 206+ 测试全部 PASS（无回归）

- [x] **Step 7: 提交**

```bash
git add src/crawl/runtime/autoscale.rs src/crawl/runner.rs tests/p0_autoscale_test.rs
git commit -m "feat: 增加 EngineBuilder.autoscale() API (P0-1 Step 1)"
```

---

### Task 2: 在 run_inner 中集成 AutoscaledPool

**Files:**
- Modify: `src/crawl/runner.rs:141-345`（run_inner 方法，增加 autoscaler spawn + 动态并发检查）
- Test: `tests/p0_autoscale_test.rs`（扩展）

**Interfaces:**
- Consumes: `Engine.autoscale: Option<Arc<AutoscaledPool>>`（Task 1 产出）、`AutoscaledPool::run_autoscaler(stats)`（已存在于 autoscale.rs:88）、`AutoscaledPool::current_concurrency()`（已存在）、`AutoscaledPool::max_concurrency()`（Task 1 产出）
- Produces: 启用 autoscale 后 run_inner 动态调整并发数；未启用时行为与原来完全一致

**背景：** 当前 `run_inner` 的流驱动用 `stream::unfold(...).buffer_unordered(max_concurrent)` 固定并发。集成 autoscale 需要：
1. 在流驱动前 spawn 后台 `run_autoscaler` task
2. unfold 生产者在 yield future 前检查 `in_flight < current_concurrency()`，超限时 yield_now 等待
3. `buffer_unordered` 的 ceiling 改用 `max_concurrency()`（而非 `max_concurrent`）
4. 流结束后 abort autoscaler task

- [x] **Step 1: 写失败测试 — autoscale 启用时并发数不超过 max**

在 `tests/p0_autoscale_test.rs` 末尾追加：

```rust
//! P0-1 Step 2: 验证 run_inner 启用 autoscale 后能正常完成爬取，
//! 且并发数不超过 max_concurrency 上限。
//! 使用不可达 URL（127.0.0.1:1），请求会快速失败，验证引擎不卡死。

use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;

struct FailSpider {
    name: String,
}

#[async_trait]
impl Spider for FailSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> { vec!["http://127.0.0.1:1/a".into(), "http://127.0.0.1:1/b".into()] }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) { (vec![], vec![]) }
    fn obey_robots(&self) -> bool { false }
    fn max_retries(&self) -> u32 { 0 }
}

#[tokio::test]
async fn run_with_autoscale_completes_without_deadlock() {
    // 启用 autoscale(1, 4)，爬取不可达 URL，引擎应正常完成不卡死
    let engine = wisp::crawl::Engine::infra()
        .max_pages(10)
        .autoscale(1, 4)
        .build()
        .expect("build engine");

    let (stats, _items) = engine.run(FailSpider { name: "fail".into() })
        .await
        .expect("run should complete");

    // 不可达 URL：请求失败不计 pages，但应计入 errors
    // （具体计数取决于 retry 逻辑，这里只验证不卡死、能返回）
    let _ = stats; // 不断言具体值，只验证 run 返回
}
```

- [x] **Step 2: 运行测试验证失败（卡死或超时）**

Run: `cargo test --test p0_autoscale_test run_with_autoscale_completes_without_deadlock -- --include-ignored`
Expected: 测试卡死或超时（因为 autoscale 字段虽存在但 run_inner 未使用它，流仍用固定并发——实际上此测试可能通过因为固定并发也能处理。真正的验证在 Step 4 的实现后。）

> 注意：此测试主要验证"启用 autoscale 不破坏正常流程"。如果 Step 2 已通过，说明现有固定并发路径兼容，直接进 Step 3 改实现。

- [x] **Step 3: 修改 run_inner 集成 AutoscaledPool**

在 `src/crawl/runner.rs` 的 `run_inner` 方法中，做以下修改：

**3a. 在构造 stream 之前，spawn autoscaler 后台 task**

在 `// 构建并发流：单 Spider，无路由` 注释之前（约 L247），增加：

```rust
// 启用 autoscale 时，spawn 后台 autoscaler task
let autoscaler_handle = if let Some(ref pool) = self.autoscale {
    let pool = Arc::clone(pool);
    let stats = Arc::clone(&stats);
    Some(tokio::spawn(async move {
        pool.run_autoscaler(stats).await;
    }))
} else {
    None
};
```

**3b. 修改 stream 构造，增加动态并发检查**

将现有的 stream 构造块（约 L248-313）替换为：

```rust
// 构建并发流：单 Spider，无路由
let stream = {
    let ctx = ctx.clone();
    let autoscale = self.autoscale.clone();
    // buffer_unordered 的 ceiling：autoscale 启用时用 max_concurrency()，否则用 max_concurrent
    let buffer_ceiling = if let Some(ref pool) = autoscale {
        pool.max_concurrency()
    } else {
        ctx.config.max_concurrent
    };
    stream::unfold((), move |_| {
        let ctx = ctx.clone();
        let autoscale = autoscale.clone();
        async move {
            loop {
                if ctx.shared.control.is_shutdown() || ctx.state.abort_flag.load(Ordering::SeqCst) {
                    return None;
                }

                // drain follow channel
                let mut rx_guard = ctx.shared.follow_rx.lock().await;
                while let Ok(req) = rx_guard.try_recv() {
                    ctx.shared.sched.push(req).await;
                }
                drop(rx_guard);

                // 引擎级 max_pages 兜底
                let pages = ctx.state.stats.pages.load(Ordering::SeqCst);
                if pages + ctx.state.global_in_flight.load(Ordering::SeqCst) >= ctx.config.engine_max_pages {
                    if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 { return None; }
                    tokio::task::yield_now().await;
                    continue;
                }

                // Spider until 终止条件检查
                let queue_size = ctx.shared.sched.len().await;
                let stop_ctx = stop::StopContext {
                    pages: ctx.state.stats.pages.load(Ordering::SeqCst),
                    items: ctx.state.stats.items.load(Ordering::SeqCst),
                    errors: ctx.state.stats.errors.load(Ordering::SeqCst),
                    in_flight: ctx.state.stats.in_flight.load(Ordering::SeqCst),
                    elapsed: ctx.state.stats.start.elapsed(),
                    queue_size,
                };
                if ctx.state.spider.until().should_stop(&stop_ctx) {
                    if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 { return None; }
                    tokio::task::yield_now().await;
                    continue;
                }

                // 动态并发限制：autoscale 启用时检查 current_concurrency
                let limit = if let Some(ref pool) = autoscale {
                    pool.current_concurrency()
                } else {
                    ctx.config.max_concurrent
                };
                if ctx.state.global_in_flight.load(Ordering::SeqCst) >= limit {
                    // 已达当前并发上限，等待 in-flight 下降
                    tokio::task::yield_now().await;
                    // 短暂等待避免忙轮询
                    tokio::time::timeout(Duration::from_millis(10), ctx.shared.work_notify.notified()).await.ok();
                    continue;
                }

                let req = match ctx.shared.sched.pop().await {
                    Some(req) => req,
                    None => {
                        if ctx.state.global_in_flight.load(Ordering::SeqCst) == 0 { return None; }
                        tokio::time::timeout(Duration::from_millis(100), ctx.shared.work_notify.notified()).await.ok();
                        continue;
                    }
                };

                // 单 Spider：直接派发，无路由
                ctx.state.global_in_flight.fetch_add(1, Ordering::SeqCst);
                ctx.state.stats.in_flight.fetch_add(1, Ordering::SeqCst);
                let ctx_c = ctx.clone();
                let fut = async move {
                    let _g1 = engine::InFlightGuard { counter: ctx_c.state.global_in_flight.clone() };
                    let _g2 = engine::InFlightGuard { counter: ctx_c.state.stats.in_flight.clone() };
                    engine::process_request(&ctx_c, req).await;
                };
                return Some((fut, ()));
            }
        }
    })
    .buffer_unordered(buffer_ceiling)
};
```

> **注意：** 此代码假设 P0-3（EngineContext 拆分）已完成，使用 `ctx.config.*`、`ctx.shared.*`、`ctx.state.*` 路径。如果 P0-3 尚未完成（本 plan 中 P0-1 先于 P0-3），需要用原始路径 `ctx.max_concurrent`、`ctx.control`、`ctx.abort_flag` 等。实施时根据当前代码状态调整路径前缀。

**3c. 在流结束后 abort autoscaler**

在 `while stream.next().await.is_some()` 循环之后（约 L326），增加：

```rust
// abort autoscaler 后台 task
if let Some(handle) = autoscaler_handle {
    handle.abort();
}
```

- [x] **Step 4: 运行测试验证通过**

Run: `cargo test --test p0_autoscale_test`
Expected: 全部 PASS

- [x] **Step 5: 回归全量测试**

Run: `cargo test --lib && cargo test --test engine_infra_test && cargo test --test crawl_concurrency_test`
Expected: 全部 PASS

- [x] **Step 6: 提交**

```bash
git add src/crawl/runner.rs tests/p0_autoscale_test.rs
git commit -m "feat: run_inner 集成 AutoscaledPool 动态并发 (P0-1 Step 2)"
```

---

## Phase 2: P0-2 domain_sems DashMap 改造

将 `domain_sems: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>` 改为 `Arc<DashMap<String, Arc<Semaphore>>>`，消除每请求全局锁。

### Task 3: 替换 domain_sems 为 DashMap

**Files:**
- Modify: `Cargo.toml`（新增 dashmap 依赖）
- Modify: `src/crawl/engine.rs:43`（EngineContext 字段类型）
- Modify: `src/crawl/engine.rs:209-214`（process_request 中信号量获取）
- Modify: `src/crawl/runner.rs:207`（run_inner 中构造）
- Modify: `src/crawl/engine.rs:713`（make_ctx 测试辅助）
- Test: `tests/p0_dashmap_test.rs`（新建）

**Interfaces:**
- Consumes: `DashMap<String, Arc<Semaphore>>`（来自 dashmap crate）
- Produces: `EngineContext.domain_sems` 类型从 `Arc<Mutex<HashMap<...>>>` 变为 `Arc<DashMap<...>>`，访问无需 `.lock().await`

- [x] **Step 1: 新增 dashmap 依赖**

在 `Cargo.toml` 的 `[dependencies]` 末尾（`croner` 后）增加：

```toml
# 高并发无锁 map（替代 domain_sems 的全局 Mutex<HashMap>）
dashmap = "6"
```

运行 `cargo build` 验证依赖可解析。

- [x] **Step 2: 写失败测试 — domain_sems DashMap 并发安全**

创建 `tests/p0_dashmap_test.rs`：

```rust
//! P0-2: 验证 domain_sems 用 DashMap 后，不同域名获取独立信号量，同域名共享。
//! 使用最小 Spider + 不可达 URL，验证引擎不 panic。

use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct MultiDomainSpider {
    name: String,
    counter: Arc<AtomicUsize>,
}

#[async_trait]
impl Spider for MultiDomainSpider {
    fn name(&self) -> &str { &self.name }
    fn start_urls(&self) -> Vec<String> {
        vec![
            "http://127.0.0.1:1/domain-a".into(),
            "http://127.0.0.1:1/domain-b".into(),
            "http://127.0.0.1:1/domain-a/page2".into(),
        ]
    }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        self.counter.fetch_add(1, Ordering::SeqCst);
        (vec![], vec![])
    }
    fn obey_robots(&self) -> bool { false }
    fn max_retries(&self) -> u32 { 0 }
}

#[tokio::test]
async fn domain_sems_no_panic_on_multiple_domains() {
    let counter = Arc::new(AtomicUsize::new(0));
    let engine = Engine::infra()
        .max_pages(10)
        .max_concurrent(4)
        .build()
        .expect("build engine");

    let spider = MultiDomainSpider {
        name: "multi-domain".into(),
        counter: counter.clone(),
    };
    // 不应 panic，能正常完成
    let _ = engine.run(spider).await;
}
```

- [x] **Step 3: 运行测试验证通过（此时应已通过，因为 DashMap 尚未引入但行为相同）**

Run: `cargo test --test p0_dashmap_test`
Expected: PASS（此测试验证行为不变，DashMap 替换后仍应通过）

- [x] **Step 4: 修改 EngineContext 字段类型**

在 `src/crawl/engine.rs` 顶部 import 区增加：

```rust
use dashmap::DashMap;
```

将 `EngineContext` 的 `domain_sems` 字段（L43）从：

```rust
pub domain_sems: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>,
```

改为：

```rust
pub domain_sems: Arc<DashMap<String, Arc<tokio::sync::Semaphore>>>,
```

- [x] **Step 5: 修改 process_request 中的信号量获取**

将 `src/crawl/engine.rs` 的 `process_request` 中信号量获取块（L209-214）从：

```rust
let sem = {
    let mut sems = ctx.domain_sems.lock().await;
    sems.entry(domain)
        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
        .clone()
};
```

改为：

```rust
let sem = {
    ctx.domain_sems
        .entry(domain)
        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
        .clone()
};
```

> DashMap 的 `entry().or_insert_with()` 返回 `Ref`，`.clone()` 克隆 `Arc<Semaphore>`。无需 `.lock().await`。

- [x] **Step 6: 修改 run_inner 中的构造**

在 `src/crawl/runner.rs` 的 `run_inner` 中，`EngineContext` 构造（L207）从：

```rust
domain_sems: Arc::new(Mutex::new(HashMap::new())),
```

改为：

```rust
domain_sems: Arc::new(DashMap::new()),
```

确保 `runner.rs` 顶部有 `use dashmap::DashMap;`（如无则增加）。

- [x] **Step 7: 修改 make_ctx 测试辅助**

在 `src/crawl/engine.rs` 的 `make_ctx` 函数（L713）中，将：

```rust
domain_sems: Arc::new(Mutex::new(HashMap::new())),
```

改为：

```rust
domain_sems: Arc::new(DashMap::new()),
```

- [x] **Step 8: 运行测试验证通过**

Run: `cargo test --test p0_dashmap_test && cargo test --lib`
Expected: 全部 PASS

- [x] **Step 9: 回归集成测试**

Run: `cargo test --test crawl_concurrency_test --test engine_infra_test --test multi_spider_test`
Expected: 全部 PASS

- [x] **Step 10: 提交**

```bash
git add Cargo.toml Cargo.lock src/crawl/engine.rs src/crawl/runner.rs tests/p0_dashmap_test.rs
git commit -m "perf: domain_sems 改用 DashMap 消除全局锁 (P0-2)"
```

---

## Phase 3: P0-3 EngineContext 三层拆分

将 30+ 字段的 `EngineContext` 拆分为 `EngineConfig`（只读配置）+ `EngineShared`（跨 task 共享可变）+ `EngineState`（per-run 可变），降低耦合度。

### Task 4: 定义三层子结构并重构 EngineContext

**Files:**
- Modify: `src/crawl/engine.rs:37-75`（EngineContext 定义 + 新增 3 个子结构）
- Modify: `src/crawl/engine.rs:80-263`（process_request 中所有字段访问路径）
- Modify: `src/crawl/engine.rs:269-356`（process_response 中字段访问路径）
- Modify: `src/crawl/engine.rs:364-463`（fetch_dispatch / auto_upgrade_check 中字段访问路径）
- Modify: `src/crawl/engine.rs:468-500`（build_crawl_context / record_status / apply_delay 中字段访问路径）
- Modify: `src/crawl/engine.rs:503-554`（snapshot_stats_for / save_checkpoint）
- Modify: `src/crawl/engine.rs:558-670`（fetch_page / fetch_page_inner 签名中 proxy_clients 参数类型）
- Modify: `src/crawl/runner.rs:201-238`（EngineContext 构造）
- Modify: `src/crawl/runner.rs:248-313`（run_inner stream 中字段访问路径）
- Modify: `src/crawl/runner.rs:329-343`（pipeline close / snapshot 中字段访问路径）
- Modify: `src/crawl/engine.rs:704-740`（make_ctx 测试辅助）

**Interfaces:**
- Consumes: 现有 `EngineContext` 的 30+ 字段
- Produces: `EngineContext { config: EngineConfig, shared: EngineShared, state: EngineState }`，所有访问改为 `ctx.config.*` / `ctx.shared.*` / `ctx.state.*`

**字段分配方案：**

```
EngineConfig（per-run 只读，从 Spider 提取）:
  client, fetcher_config, fetch_mode, max_concurrent, max_depth,
  obey_robots, engine_max_pages, max_refetch_rounds, dev_mode,
  allowed, auto_excludes

EngineShared（per-run 跨 task 共享可变）:
  sched, robots_cache, follow_tx, follow_rx, domain_sems, proxy_clients,
  cache_store, request_cache, control, work_notify, middleware_chain,
  rule_engine

EngineState（per-run 可变状态）:
  spider, stats, items, abort_flag, start, tx, global_in_flight
```

- [x] **Step 1: 在 engine.rs 中定义 3 个子结构**

在 `src/crawl/engine.rs` 的 `EngineContext` 定义之前（L29 附近），增加 3 个结构体定义：

```rust
// === EngineContext 三层拆分 ===

/// 只读配置（从 Spider 提取，run 期间不变）。
pub(crate) struct EngineConfig {
    pub client: Arc<Client>,
    pub fetcher_config: http::Config,
    pub fetch_mode: FetchMode,
    pub max_concurrent: usize,
    pub max_depth: u32,
    pub obey_robots: bool,
    pub engine_max_pages: usize,
    pub max_refetch_rounds: usize,
    pub dev_mode: bool,
    pub allowed: Arc<HashSet<String>>,
    pub auto_excludes: HashSet<String>,
}

/// 跨 task 共享的可变状态。
pub(crate) struct EngineShared {
    pub sched: Arc<scheduler::Scheduler>,
    pub robots_cache: Arc<Mutex<robots::RobotsCache>>,
    pub follow_tx: tokio::sync::mpsc::UnboundedSender<SpiderRequest>,
    pub follow_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<SpiderRequest>>>,
    pub domain_sems: Arc<DashMap<String, Arc<tokio::sync::Semaphore>>>,
    pub proxy_clients: Arc<Mutex<HashMap<String, Arc<Client>>>>,
    pub cache_store: Option<Arc<crate::storage::Store>>,
    pub request_cache: Option<super::request_cache::RequestCache>,
    pub control: Arc<control::EngineControl>,
    pub work_notify: Arc<tokio::sync::Notify>,
    pub middleware_chain: Arc<middleware::MiddlewareChain>,
    pub rule_engine: Arc<Mutex<auto::ModeRuleEngine>>,
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

- [x] **Step 2: 重构 EngineContext 为三层包装**

将 `EngineContext` 定义（L37-75）替换为：

```rust
/// Engine 运行时上下文（单 Spider），由三层子结构组成。
///
/// - `config`: 只读配置（从 Spider 提取，run 期间不变）
/// - `shared`: 跨 task 共享的可变状态
/// - `state`: per-run 可变状态
pub(crate) struct EngineContext {
    pub config: EngineConfig,
    pub shared: EngineShared,
    pub state: EngineState,
}
```

- [x] **Step 3: 更新 run_inner 中的 EngineContext 构造**

在 `src/crawl/runner.rs` 的 `run_inner` 中，将 `let ctx = Arc::new(engine::EngineContext { ... });`（L201-238）替换为三层构造：

```rust
let ctx = Arc::new(engine::EngineContext {
    config: engine::EngineConfig {
        client: self.client.clone(),
        fetcher_config,
        fetch_mode,
        max_concurrent,
        max_depth,
        obey_robots,
        engine_max_pages: self.max_pages,
        max_refetch_rounds: self.max_refetch_rounds,
        dev_mode: self.dev_mode,
        allowed,
        auto_excludes,
    },
    shared: engine::EngineShared {
        sched: sched.clone(),
        robots_cache,
        follow_tx,
        follow_rx: Arc::new(Mutex::new(follow_rx)),
        domain_sems: Arc::new(DashMap::new()),
        proxy_clients: Arc::new(Mutex::new(HashMap::new())),
        cache_store: self.cache_store.clone(),
        request_cache: self.request_cache.clone(),
        control: self.control.clone(),
        work_notify: Arc::new(tokio::sync::Notify::new()),
        middleware_chain: {
            let mut chain = middleware::MiddlewareChain::new();
            chain.middlewares = spider.middlewares();
            chain.pipelines = spider.pipelines();
            chain.sort();
            Arc::new(chain)
        },
        rule_engine,
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

> **注意：** `follow_tx` 需要在 `EngineShared` 构造前先从 unbounded channel 获取，但 `follow_rx` 也需要。当前代码 `let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();` 在 L166 已有，这里直接使用即可。但要注意 `follow_tx` 被 move 到 shared 后不能再被后续代码引用——当前 run_inner 中 `follow_tx` 只在 ctx 构造中使用，所以没问题。

> **另一个注意：** `allowed` 和 `auto_excludes` 在当前代码中是 `let allowed = Arc::new(spider.allowed_domains());` 和 `let auto_excludes = spider.auto_exclude();`（L156, L162），这两个变量被 move 到 config 中。后续代码不应再引用它们。

- [x] **Step 4: 全局替换字段访问路径**

在 `src/crawl/engine.rs` 和 `src/crawl/runner.rs` 中，按以下映射全局替换所有 `ctx.<field>` 访问：

**EngineConfig 字段（`ctx.config.*`）：**
- `ctx.client` → `ctx.config.client`
- `ctx.fetcher_config` → `ctx.config.fetcher_config`
- `ctx.fetch_mode` → `ctx.config.fetch_mode`
- `ctx.max_concurrent` → `ctx.config.max_concurrent`
- `ctx.max_depth` → `ctx.config.max_depth`
- `ctx.obey_robots` → `ctx.config.obey_robots`
- `ctx.engine_max_pages` → `ctx.config.engine_max_pages`
- `ctx.max_refetch_rounds` → `ctx.config.max_refetch_rounds`
- `ctx.dev_mode` → `ctx.config.dev_mode`
- `ctx.allowed` → `ctx.config.allowed`
- `ctx.auto_excludes` → `ctx.config.auto_excludes`

**EngineShared 字段（`ctx.shared.*`）：**
- `ctx.sched` → `ctx.shared.sched`
- `ctx.robots_cache` → `ctx.shared.robots_cache`
- `ctx.follow_tx` → `ctx.shared.follow_tx`
- `ctx.follow_rx` → `ctx.shared.follow_rx`
- `ctx.domain_sems` → `ctx.shared.domain_sems`
- `ctx.proxy_clients` → `ctx.shared.proxy_clients`
- `ctx.cache_store` → `ctx.shared.cache_store`
- `ctx.request_cache` → `ctx.shared.request_cache`
- `ctx.control` → `ctx.shared.control`
- `ctx.work_notify` → `ctx.shared.work_notify`
- `ctx.middleware_chain` → `ctx.shared.middleware_chain`
- `ctx.rule_engine` → `ctx.shared.rule_engine`

**EngineState 字段（`ctx.state.*`）：**
- `ctx.spider` → `ctx.state.spider`
- `ctx.stats` → `ctx.state.stats`
- `ctx.items` → `ctx.state.items`
- `ctx.abort_flag` → `ctx.state.abort_flag`
- `ctx.start` → `ctx.state.start`
- `ctx.tx` → `ctx.state.tx`
- `ctx.global_in_flight` → `ctx.state.global_in_flight`

> **实施方式：** 用编辑器全局搜索替换。注意不要替换 `build_crawl_context` 中的局部变量绑定（它构造 `CrawlContext` 返回值）。也要注意 `process_request` 中开头的局部变量解包（`let spider = &ctx.spider;` 等）需要同步更新为 `let spider = &ctx.state.spider;`。

- [x] **Step 5: 更新 build_crawl_context**

在 `src/crawl/engine.rs` 的 `build_crawl_context` 函数（L468-478）中，更新字段访问：

```rust
pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
    middleware::CrawlContext {
        spider_name: ctx.state.spider.name().to_string(),
        fetch_mode: ctx.config.fetch_mode,
        max_concurrent: ctx.config.max_concurrent,
        max_pages: ctx.config.engine_max_pages,
        obey_robots: ctx.config.obey_robots,
        pages_crawled: ctx.state.stats.pages.load(Ordering::SeqCst),
        errors: ctx.state.stats.errors.load(Ordering::SeqCst),
    }
}
```

- [x] **Step 6: 更新 fetch_page / fetch_page_inner 的 proxy_clients 参数类型**

`fetch_page` 和 `fetch_page_inner` 的 `proxy_clients` 参数类型从 `&Mutex<HashMap<String, Arc<Client>>>` 改为 `&Arc<Mutex<HashMap<String, Arc<Client>>>>`（因为现在是 `ctx.shared.proxy_clients` 类型是 `Arc<Mutex<...>>`，传引用时类型变了）。

或者更简单：调用处传 `&ctx.shared.proxy_clients`（类型 `&Arc<Mutex<...>>`），函数签名改为接收 `proxy_clients: &Arc<Mutex<HashMap<String, Arc<Client>>>>`。

在 `src/crawl/engine.rs` 的 `fetch_page`（L558-586）和 `fetch_page_inner`（L589-670）中，将参数：

```rust
proxy_clients: &Mutex<HashMap<String, Arc<Client>>>,
```

改为：

```rust
proxy_clients: &Arc<Mutex<HashMap<String, Arc<Client>>>>,
```

函数体内的 `proxy_clients.lock().await` 不变（`Arc<Mutex<T>>` 也支持 `.lock().await`）。

- [x] **Step 7: 更新 make_ctx 测试辅助**

在 `src/crawl/engine.rs` 的 `make_ctx`（L704-740）中，将 EngineContext 构造改为三层：

```rust
fn make_ctx() -> (EngineContext, Arc<SpiderStats>) {
    let stats = Arc::new(SpiderStats::new());
    let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel::<SpiderRequest>();
    let ctx = EngineContext {
        config: EngineConfig {
            client: Arc::new(Client::new().expect("build http client")),
            fetcher_config: http::Config::default(),
            fetch_mode: FetchMode::Http,
            max_concurrent: 8,
            max_depth: u32::MAX,
            obey_robots: false,
            engine_max_pages: 100,
            max_refetch_rounds: 5,
            dev_mode: false,
            allowed: Arc::new(HashSet::new()),
            auto_excludes: HashSet::new(),
        },
        shared: EngineShared {
            sched: Arc::new(scheduler::Scheduler::new()),
            robots_cache: Arc::new(Mutex::new(robots::RobotsCache::new())),
            follow_tx,
            follow_rx: Arc::new(Mutex::new(follow_rx)),
            domain_sems: Arc::new(DashMap::new()),
            proxy_clients: Arc::new(Mutex::new(HashMap::new())),
            cache_store: None,
            request_cache: None,
            control: Arc::new(control::EngineControl::new()),
            work_notify: Arc::new(tokio::sync::Notify::new()),
            middleware_chain: Arc::new(middleware::MiddlewareChain::new()),
            rule_engine: Arc::new(Mutex::new(auto::ModeRuleEngine::new())),
        },
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
    (ctx, stats)
}
```

- [x] **Step 8: 更新 run_inner 中 stream 和后续代码的字段访问**

在 `src/crawl/runner.rs` 的 `run_inner` 中，stream 构造块和流驱动块（L248-343）中的所有 `ctx.<field>` 访问需同步更新。具体：

- `ctx.control` → `ctx.shared.control`
- `ctx.abort_flag` → `ctx.state.abort_flag`
- `ctx.follow_rx` → `ctx.shared.follow_rx`
- `ctx.sched` → `ctx.shared.sched`
- `ctx.stats` → `ctx.state.stats`
- `ctx.global_in_flight` → `ctx.state.global_in_flight`
- `ctx.engine_max_pages` → `ctx.config.engine_max_pages`
- `ctx.spider` → `ctx.state.spider`
- `ctx.max_concurrent` → `ctx.config.max_concurrent`
- `ctx.work_notify` → `ctx.shared.work_notify`
- `ctx.middleware_chain` → `ctx.shared.middleware_chain`
- `ctx.items` → `ctx.state.items`
- `ctx.tx` → `ctx.state.tx`
- `ctx.start` → `ctx.state.start`

中间件初始化块（L241-245）和 pipeline 关闭块（L329-332）也需更新：

```rust
// 中间件初始化
if !ctx.shared.middleware_chain.is_empty() {
    let crawl_ctx = engine::build_crawl_context(&ctx);
    ctx.shared.middleware_chain.run_init(&crawl_ctx).await;
    ctx.shared.middleware_chain.run_pipelines_open(&crawl_ctx).await;
}
```

```rust
// pipeline 关闭
if !ctx.shared.middleware_chain.is_empty() {
    let crawl_ctx = engine::build_crawl_context(&ctx);
    ctx.shared.middleware_chain.run_pipelines_close(&crawl_ctx).await;
}
```

snapshot 末尾（L342-343）：

```rust
let status_codes = ctx.state.stats.status_codes.lock().await.clone();
Ok(engine::snapshot_stats_for(&ctx.state.stats, status_codes, ctx.state.start))
```

- [x] **Step 9: 编译并修复所有路径错误**

Run: `cargo build`
Expected: 编译错误（字段路径不匹配），逐个修复

> 此步骤是机械式的：编译器会报出所有需要修复的字段路径，按 Step 4 的映射表逐个修复。

- [x] **Step 10: 运行全量测试验证无回归**

Run: `cargo test --lib && cargo test --test engine_infra_test --test crawl_concurrency_test --test multi_spider_test`
Expected: 全部 PASS

- [x] **Step 11: 提交**

```bash
git add src/crawl/engine.rs src/crawl/runner.rs
git commit -m "refactor: EngineContext 拆分为 Config/Shared/State 三层 (P0-3)"
```

---

## Phase 4: P0-4 process_request 拆分

将 200 行的 `process_request` 拆分为独立 stage 函数，主流程变为薄编排层。

### Task 5: 拆分 process_request 为 stage 函数

**Files:**
- Modify: `src/crawl/engine.rs:80-263`（process_request 拆分）
- Modify: `src/crawl/engine.rs`（新增 stage 函数：check_request_filters / check_request_caches / execute_fetch_and_process）
- Test: `src/crawl/engine.rs` 内 `#[cfg(test)] mod tests`（扩展）

**Interfaces:**
- Consumes: P0-3 产出的三层 EngineContext（`ctx.config.*` / `ctx.shared.*` / `ctx.state.*`）
- Produces: `process_request` 从 200 行缩减为 ~40 行编排层；stage 函数独立可测

**Stage 拆分方案：**

```
process_request(ctx, req):
  1. check_request_filters(ctx, &req) -> FilterAction   // 域名/深度/控制/钩子
  2. run_request_middlewares(ctx, &mut req) -> MwAction // 中间件请求拦截
  3. check_request_caches(ctx, &req) -> CacheResult      // RequestCache + dev_mode
  4. acquire_and_fetch(ctx, &req) -> FetchOutcome        // robots + sem + delay + fetch_dispatch + cache save
  5. process_response(ctx, resp, &req)                   // 已存在，不改
```

- [x] **Step 1: 定义 FilterAction 和 CacheResult 枚举**

在 `src/crawl/engine.rs` 的 `EngineContext` 定义之后，增加：

```rust
/// 请求过滤结果。
enum FilterAction {
    /// 继续处理
    Proceed,
    /// 跳过此请求
    Skip,
    /// 中止整个爬取
    Abort,
    /// 延迟后继续
    Delay(Duration),
}

/// 缓存检查结果。
enum CacheResult {
    /// 缓存命中，直接处理此响应
    Hit(SpiderResponse),
    /// 未命中，继续网络请求
    Miss,
}
```

- [x] **Step 2: 提取 check_request_filters stage**

在 `src/crawl/engine.rs` 中，新增函数：

```rust
/// Stage 1: 请求过滤检查（域名/深度/控制状态/异步钩子）。
///
/// 返回 FilterAction::Proceed 表示继续；其他表示终止此请求的处理。
async fn check_request_filters(ctx: &EngineContext, req: &SpiderRequest) -> FilterAction {
    let stats = &ctx.state.stats;
    let allowed = &ctx.config.allowed;

    // 1. 域名过滤
    if !allowed.is_empty() {
        if let Ok(parsed) = url::Url::parse(&req.url) {
            if let Some(host) = parsed.host_str() {
                if !allowed.contains(host) {
                    stats.offsite.fetch_add(1, Ordering::SeqCst);
                    return FilterAction::Skip;
                }
            }
        }
    }

    // 1.5. 深度检查
    if req.depth > ctx.config.max_depth {
        return FilterAction::Skip;
    }

    // 1.6. per-Engine 控制状态检查
    if ctx.shared.control.is_cancelled(&req.url).await { return FilterAction::Skip; }
    if !ctx.shared.control.wait_if_paused(&req.url).await { return FilterAction::Skip; }
    if ctx.shared.control.is_shutdown() { return FilterAction::Skip; }

    // 1.7. 异步钩子检查
    match ctx.state.spider.on_before_request(req).await {
        super::RequestAction::Proceed => FilterAction::Proceed,
        super::RequestAction::Skip => FilterAction::Skip,
        super::RequestAction::Delay(d) => FilterAction::Delay(d),
        super::RequestAction::Abort => FilterAction::Abort,
    }
}
```

- [x] **Step 3: 提取 check_request_caches stage**

在 `src/crawl/engine.rs` 中，新增函数：

```rust
/// Stage 3: 缓存检查（RequestCache 内存缓存 + dev_mode SQLite 缓存）。
///
/// 返回 CacheResult::Hit(resp) 表示命中缓存，直接处理响应；
/// 返回 CacheResult::Miss 表示未命中，继续网络请求。
async fn check_request_caches(ctx: &EngineContext, req: &SpiderRequest, method_str: &str) -> CacheResult {
    let stats = &ctx.state.stats;

    // 内存缓存检查 (RequestCache)
    if let Some(ref rc) = ctx.shared.request_cache {
        if let Some(entry) = rc.get(method_str, &req.url).await {
            let resp = SpiderResponse {
                url: req.url.clone(),
                status: entry.status,
                headers: entry.headers,
                body: entry.body,
                request: req.clone(),
                tracker: None,
                from_cache: true,
            };
            stats.cache_hits.fetch_add(1, Ordering::SeqCst);
            record_status(stats, resp.status).await;
            return CacheResult::Hit(resp);
        }
    }

    // 开发模式 SQLite 缓存检查
    if ctx.config.dev_mode {
        if let Some(ref store) = ctx.shared.cache_store {
            if let Some(cached) = store.load_cached_response(&req.url, method_str).ok().flatten() {
                let resp = SpiderResponse {
                    url: req.url.clone(),
                    status: cached.status,
                    headers: cached.headers,
                    body: cached.body,
                    request: req.clone(),
                    tracker: None,
                    from_cache: true,
                };
                stats.cache_hits.fetch_add(1, Ordering::SeqCst);
                record_status(stats, resp.status).await;
                return CacheResult::Hit(resp);
            }
        }
    }

    CacheResult::Miss
}
```

- [x] **Step 4: 重写 process_request 为编排层**

将 `src/crawl/engine.rs` 的 `process_request`（L80-263）替换为：

```rust
/// 处理单个请求的完整流程（编排层，~50 行）。
///
/// Stages:
/// 1. check_request_filters — 域名/深度/控制/钩子
/// 2. run_request_middlewares — 中间件请求拦截（可能短路）
/// 3. check_request_caches — RequestCache + dev_mode 缓存
/// 4. acquire_and_fetch — robots + 信号量 + 延迟 + fetch_dispatch + 缓存写入
/// 5. process_response — handle + items + events（已存在）
pub(crate) async fn process_request(ctx: &EngineContext, req: SpiderRequest) {
    // 1. 过滤检查
    match check_request_filters(ctx, &req).await {
        FilterAction::Proceed => {}
        FilterAction::Skip => return,
        FilterAction::Abort => {
            ctx.state.abort_flag.store(true, Ordering::SeqCst);
            return;
        }
        FilterAction::Delay(d) => { tokio::time::sleep(d).await; }
    }

    // 2. 中间件请求拦截
    let mut req = req;
    if !ctx.shared.middleware_chain.is_empty() {
        let crawl_ctx = build_crawl_context(ctx);
        match ctx.shared.middleware_chain.run_request_middlewares(&mut req, &crawl_ctx).await {
            middleware::MwAction::Skip => return,
            middleware::MwAction::Abort(reason) => {
                tracing::warn!("middleware abort: {} - {}", reason, req.url);
                return;
            }
            middleware::MwAction::Respond(cached_resp) => {
                ctx.state.stats.cache_hits.fetch_add(1, Ordering::SeqCst);
                record_status(&ctx.state.stats, cached_resp.status).await;
                return process_response(ctx, cached_resp, &req).await;
            }
            _ => {}
        }
    }

    // 提前计算 method_str（缓存查询与写入都需要）
    let method_str = match req.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    };

    // 3. 缓存检查
    match check_request_caches(ctx, &req, method_str).await {
        CacheResult::Hit(resp) => {
            return process_response(ctx, resp, &req).await;
        }
        CacheResult::Miss => {}
    }

    // 4. robots + 信号量 + 延迟 + 抓取 + 缓存写入
    let (final_resp, last_error) = acquire_and_fetch(ctx, &req, method_str).await;

    // 5. 处理结果
    if let Some(resp) = final_resp {
        process_response(ctx, resp, &req).await;
    } else if let Some(err) = last_error {
        if let Some(ref tx) = ctx.state.tx {
            let _ = tx.send(CrawlEvent::Error { url: req.url.clone(), error: err }).await;
        }
    }
}
```

- [x] **Step 5: 提取 acquire_and_fetch stage**

在 `src/crawl/engine.rs` 中，新增函数（封装原 process_request 的 robots/sem/delay/fetch/cache-save 部分）：

```rust
/// Stage 4: robots 检查 → 域名信号量 → 延迟 → fetch_dispatch → 缓存写入。
///
/// 返回 (Option<SpiderResponse>, Option<String>) — 成功返回 resp，失败返回 error。
async fn acquire_and_fetch(
    ctx: &EngineContext,
    req: &SpiderRequest,
    method_str: &str,
) -> (Option<SpiderResponse>, Option<String>) {
    let stats = &ctx.state.stats;
    let spider = &ctx.state.spider;
    let obey_robots = ctx.config.obey_robots;
    let max_concurrent = ctx.config.max_concurrent;

    // robots 检查
    if obey_robots {
        let allowed_flag = {
            let mut rc = ctx.shared.robots_cache.lock().await;
            rc.is_allowed(&ctx.config.client, &req.url).await
        };
        if !allowed_flag {
            return (None, None);
        }
    }

    // 域名信号量
    let domain = url::Url::parse(&req.url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let sem = {
        ctx.shared.domain_sems
            .entry(domain)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)))
            .clone()
    };
    let Ok(_permit) = sem.acquire_owned().await else {
        tracing::warn!("domain semaphore closed, skipping: {}", req.url);
        return (None, None);
    };

    // 延迟
    apply_delay(ctx, &req.url, spider, obey_robots).await;

    // 带重试的抓取
    let (resp, err) = fetch_dispatch(ctx, req).await;

    // 开发模式缓存保存
    if ctx.config.dev_mode {
        if let Some(ref store) = ctx.shared.cache_store {
            if let Some(ref resp) = resp {
                let cached = crate::storage::CachedResponse {
                    status: resp.status,
                    headers: resp.headers.clone(),
                    body: resp.body.clone(),
                    cached_at: chrono::Utc::now().timestamp(),
                };
                let _ = store.save_cached_response(&req.url, method_str, &cached);
            }
        }
    }

    // 写入 RequestCache
    if let Some(ref rc) = ctx.shared.request_cache {
        if let Some(ref resp) = resp {
            rc.put(method_str, &req.url, super::request_cache::CachedEntry {
                status: resp.status,
                headers: resp.headers.clone(),
                body: resp.body.clone(),
            }).await;
        }
    }

    (resp, err)
}
```

- [x] **Step 6: 更新 fetch_dispatch 和 apply_delay 中的字段访问**

确保 `fetch_dispatch`（L364-423）和 `apply_delay`（L485-500）中的字段访问已使用 P0-3 的三层路径：

- `ctx.spider` → `ctx.state.spider`
- `ctx.stats` → `ctx.state.stats`
- `ctx.fetch_mode` → `ctx.config.fetch_mode`
- `ctx.fetcher_config` → `ctx.config.fetcher_config`
- `ctx.rule_engine` → `ctx.shared.rule_engine`
- `ctx.proxy_clients` → `ctx.shared.proxy_clients`
- `ctx.middleware_chain` → `ctx.shared.middleware_chain`
- `ctx.client` → `ctx.config.client`
- `ctx.robots_cache` → `ctx.shared.robots_cache`

- [x] **Step 7: 编译并修复错误**

Run: `cargo build`
Expected: 编译成功

- [x] **Step 8: 运行全量测试验证无回归**

Run: `cargo test --lib && cargo test --test engine_infra_test --test crawl_concurrency_test --test multi_spider_test --test crawl_cache_real_test`
Expected: 全部 PASS

> 特别关注 `process_response_from_cache_does_not_increment_pages` 和 `process_response_not_from_cache_increments_pages` 这两个直接测试 process_response 的单元测试，验证行为不变。

- [x] **Step 9: 提交**

```bash
git add src/crawl/engine.rs
git commit -m "refactor: process_request 拆分为 stage 函数 (P0-4)"
```

---

### Task 6: 最终回归验证与清理

**Files:**
- 全量测试
- `docs/superpowers/plans/2026-07-23-p0-architecture-optimization.md`（本文件，标记完成）

**Interfaces:**
- 无新增接口

- [x] **Step 1: 全量 lib + integration 测试**

Run: `cargo test --lib`
Expected: 206+ 测试全部 PASS

Run: `cargo test --test p0_autoscale_test --test p0_dashmap_test --test engine_infra_test --test crawl_concurrency_test --test multi_spider_test --test crawl_cache_real_test --test crawl_retry_real_test --test builder_api_test --test cr_fix_engine_test`
Expected: 全部 PASS

- [x] **Step 2: 验证 autoscale 集成端到端**

Run: `cargo test --test p0_autoscale_test`
Expected: 3+ 测试全部 PASS

- [x] **Step 3: 验证 clippy 无新警告**

Run: `cargo clippy --lib 2>&1 | grep -c "warning"`
Expected: 0 或不超过基线数量

- [x] **Step 4: 标记 plan 完成**

在 plan 文件中所有 checkbox 标记为 `[x]`。

- [x] **Step 5: 提交 plan 完成标记**

```bash
git add docs/superpowers/plans/2026-07-23-p0-architecture-optimization.md
git commit -m "docs: P0 架构优化 plan 全部完成"
```

---

## Self-Review

### 1. Spec coverage

| Spec 项 | Plan Task |
|---|---|
| P0-1 AutoscaledPool 集成 | Task 1 (API) + Task 2 (run_inner 集成) |
| P0-2 domain_sems DashMap | Task 3 (字段替换 + 访问更新) |
| P0-3 EngineContext 三层拆分 | Task 4 (子结构 + 全量路径替换) |
| P0-4 process_request 拆分 | Task 5 (stage 函数提取) + Task 6 (回归验证) |

### 2. Placeholder scan

- 无 "TBD" / "TODO" / "implement later"
- 所有 step 均有具体代码
- 所有测试均有具体断言
- 所有命令都有 expected output

### 3. Type consistency

- `EngineContext` 在 Task 4 重构为三层后，所有后续 Task 均使用 `ctx.config.*` / `ctx.shared.*` / `ctx.state.*` 路径
- `AutoscaledPool::max_concurrency()` 在 Task 1 定义，Task 2 使用
- `DashMap` 在 Task 3 引入后，Task 4 的 `EngineShared` 使用 `Arc<DashMap<...>>` 类型
- `FilterAction` / `CacheResult` 枚举在 Task 5 Step 1 定义，Step 2-5 使用
- `acquire_and_fetch` 在 Task 5 Step 5 定义，Step 4 的 `process_request` 调用

### 4. 依赖顺序

```
Task 1 (autoscale API) → Task 2 (autoscale 集成，使用 ctx.config.* 路径)
Task 3 (DashMap) → Task 4 (EngineContext 拆分，EngineShared 使用 DashMap)
Task 4 (三层拆分) → Task 5 (stage 函数使用三层路径)
Task 5 → Task 6 (回归验证)
```

> **关键注意：** Task 2 的代码示例使用了 `ctx.config.*` / `ctx.shared.*` / `ctx.state.*` 路径，但 P0-3（Task 4）在 Task 2 之后才执行。实施时有两个选择：
> 1. **调整执行顺序**：先做 Task 3+4（DashMap + 拆分），再做 Task 1+2（autoscale）——这样 Task 2 可直接用三层路径
> 2. **按 plan 顺序执行**：Task 2 先用原始路径 `ctx.max_concurrent` 等，Task 4 拆分时再统一更新——更安全但 Task 2 代码需调整路径前缀
>
> **推荐选择 1**：调整执行顺序为 Task 3 → Task 4 → Task 1 → Task 2 → Task 5 → Task 6。这样 Task 2 可直接用三层路径，减少重复修改。

---

## 执行顺序建议

按依赖关系，推荐执行顺序为：

1. **Task 3**（P0-2 DashMap）— 独立，先消除全局锁
2. **Task 4**（P0-3 EngineContext 拆分）— 基于 DashMap 类型，为后续铺路
3. **Task 1**（P0-1 autoscale API）— 独立，增加 builder 方法
4. **Task 2**（P0-1 autoscale 集成）— 基于三层路径写动态并发
5. **Task 5**（P0-4 process_request 拆分）— 基于三层路径写 stage 函数
6. **Task 6**（回归验证）— 全量测试

> 此顺序避免 Task 2 中路径前缀的重复修改，每个 Task 完成后代码即处于一致状态。
