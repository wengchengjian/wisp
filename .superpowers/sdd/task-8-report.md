# Task 8 Report: Engine 重构为 buffer_unordered 真并发

## 状态

DONE_WITH_CONCERNS

## 提交 hash

```
7b431d5 refactor: Engine 重构为 buffer_unordered 真并发 + per-domain 信号量
```

## 修改的文件清单

### 1. `src/crawl/mod.rs`（修改）

**imports 区追加**（line 10-15）：
- `use std::sync::atomic::{AtomicUsize, Ordering};`
- `use std::sync::Arc;`
- `use futures::stream::{self, StreamExt};`
- `use tokio::sync::Mutex;`

（已有的 `HashMap`/`HashSet`、`Duration` 保持不变，未重复 import。）

**替换部分**（原 line 101-218 的 `Engine` struct + impl + `fetch_page` 方法）：

- 新增 `EngineConfig` struct + `Default` impl，集中管理 `max_pages` / `max_concurrent`。
- `Engine` struct 字段改为 `spider: S` + `config: EngineConfig`，并新增 `max_concurrent` builder 方法。
- `run()` 完全重写：
  - 提前提取 `max_pages`/`max_concurrent`/`obey_robots`/`allowed_domains`/`start_urls`/`fetcher_config`，避免 `self` 部分移动问题。
  - `spider` 包成 `Arc<S>`，`Scheduler` / `RobotsCache` / `Client` / `allowed` 全部 `Arc` 化以供并发任务共享。
  - 用 `tokio::sync::mpsc::unbounded_channel` 回灌 follow requests。
  - 统计用 `AtomicUsize`（`stats_items`/`stats_pages`/`stats_errors`）。
  - 用 `HashMap<String, Arc<Semaphore>>` 做 per-domain 节流（每个域信号量许可数 = `max_concurrent`）。
  - 核心用 `stream::unfold(() , ...)` 产生 future，再 `.buffer_unordered(max_concurrent)` 真并发执行。
  - unfold 内每轮：先 drain follow channel → 检查 page budget → `sched.pop().await?` → 构造一个 `async move { ... }` future（包含 domain filter / robots check / per-domain sem acquire / fetch / parse / on_item / 回灌 follow）→ `Some((fut, ()))`。
  - 主循环 `tokio::pin!(stream); while stream.next().await.is_some() {}` 驱动 stream 完成。
- `fetch_page` 从 `&self` 方法改为自由函数 `async fn fetch_page(client: &Client, req: &SpiderRequest) -> Result<SpiderResponse>`（因为 `Arc<S>` 下不再需要 `&self`）。

**保留不变**：模块声明、`Method`、`SpiderRequest`、`SpiderResponse`、`Spider` trait、`CrawlStats`。

### 2. `tests/crawl_concurrency_test.rs`（新建）

完全按 brief 给的代码原样创建。`ConcurrencySpider` 实现 `Spider`，10 个 `httpbin.org/delay/0.1` URL，`concurrent_requests()=4`。`test_max_concurrent_respected` 标记 `#[ignore = "requires network access to httpbin.org"]`。

## 测试结果

### `cargo check`
- exit code: 0
- 编译通过
- 5 个 warnings，全部是预先存在的（`browser/mod.rs`、`scraper/mod.rs`、`page/mod.rs`、`challenge/mod.rs`、`storage/mod.rs`），与本次修改无关
- `src/crawl/mod.rs` 自身无 warning

### `cargo check --tests`
- exit code: 0
- 集成测试文件也编译通过
- 唯一新增 warning：`tests/crawl_concurrency_test.rs:3:5: unused import: std::sync::Arc`（brief 原文如此，保持不变）

### `cargo test --lib`
- exit code: 0
- `test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
- 所有 34 个 lib 单元测试通过

## 实现说明

整体按 brief 实现，但有 **3 处偏离 brief 代码**（均为编译错误强制要求，已标注原因）：

### 偏离 1：移除 `.map(|(fut, _)| fut)` 一行（brief 代码 bug）

**brief 原文**：
```rust
stream::unfold((), move |_| { ... async move { ... Some((fut, ())) } })
    .map(|(fut, _)| fut)     // ← brief 这一行是错的
    .buffer_unordered(max_concurrent)
```

**问题**：`stream::unfold` 的 closure 返回 `Option<(Item, State)>`，其中 `Item` 就是 unfold stream yield 的类型。brief 中 closure 返回 `Some((fut, ()))`，所以 unfold 已经直接 yield `fut`（async block）。后续 `.map(|(fut, _)| fut)` 试图把 `fut`（已经是 async block）当作 `(fut, _)` 元组解构，编译报错：
```
error[E0308]: mismatched types
   expected `async` block, found `(_, _)`
```

**修复**：直接删除 `.map(|(fut, _)| fut)` 一行，unfold 直接喂给 `buffer_unordered`。

### 偏离 2：用 `tokio::pin!` 替代直接 `stream.next()`（brief 遗漏 pin）

**brief 原文**：
```rust
while stream.next().await.is_some() {}
```

**问题**：`stream` 类型是 `BufferUnordered<Unfold<..., async block>>`，其中 unfold 内部的 `async move { ... }` block 是 `!Unpin`，导致整个 stream `!Unpin`。`StreamExt::next` 要求 `Self: Unpin`，编译报错：
```
error[E0277]: `{async block@...}` cannot be unpinned
```

**修复**：在 while 循环前加 `tokio::pin!(stream);`，把 `stream` shadow 成 `Pin<&mut Stream>`（`tokio::pin!` 比 `std::pin::pin!` 在这里更可靠地完成 shadow + Unpin 满足）。同时把 `let mut stream = { ... }` 改成 `let stream = { ... }`（去掉 `mut`，因为 `tokio::pin!` 自己处理）。

### 偏离 3：`let mut rc = robots_cache_c.lock().await`

**brief 预见**：brief 在「常见编译问题」里提到 `is_allowed` 需要 `&mut self`，并说 `MutexGuard` 实现 `DerefMut` 应该可行。

**实际**：`rc.is_allowed(...)` 通过 `DerefMut` 调用确实可行，但需要 `rc` 本身声明为 `mut`（因为 `DerefMut::deref_mut(&mut self)` 需要 `&mut rc`）。编译报错：
```
error[E0596]: cannot borrow `rc` as mutable, as it is not declared as mutable
```

**修复**：把 `let rc = ...` 改成 `let mut rc = ...`。这是 brief 预见的常见问题 #1 的标准修复。

## 关切点（concerns）

### 1. follow channel 可能丢消息（架构性关切，非编译问题）

当前 unfold 逻辑：
1. drain follow channel → push to scheduler
2. 检查 page budget
3. `sched.pop().await?` ← 如果返回 `None`，unfold 返回 `None`，stream 结束

**潜在问题**：如果 scheduler 为空、channel 为空，但 `buffer_unordered` 中仍有 in-flight future 正在执行（尚未发 follow），unfold 会因 `sched.pop() → None` 提前结束 stream。in-flight future 完成后发出的 follow 请求会进入 channel，但 unfold 已经停止，这些 follow 会被丢弃。

**影响场景**：
- 单 URL 种子 + `max_concurrent > 1` 时，第一批 future 还在跑（尚未发 follow），unfold 被轮询时 scheduler 已空 → 提前结束。
- 实际上 `buffer_unordered` 只在 buffer 有空位时才 poll unfold，所以此问题在「buffer 未满 + scheduler 空 + channel 空 + in-flight future 未完成」时触发。

**对 brief 测试的影响**：测试 spider 的 `parse` 返回 `(vec![], vec![])`（无 follow），所以不会触发此问题，测试能通过。但对于真实有多层 follow 的爬虫，可能丢消息。

**建议**：reviewer 可在后续 task 考虑用 `Arc<AtomicUsize>` 跟踪 in-flight future 数量，当 `sched.is_empty() && channel_empty && in_flight == 0` 时才返回 `None`。

### 2. `download_delay` 不再生效（功能丢失）

原串行 Engine 在每个 request 之间 `tokio::time::sleep(delay).await`。新并发模型中没有调用 `download_delay()`。`Spider::download_delay` trait 方法仍然存在，但 Engine 不再使用。

**影响**：用户若依赖 `download_delay` 做礼貌爬取，会失效。per-domain 信号量限制了并发数，但不提供请求间延迟。

**建议**：reviewer 可考虑在 per-domain 信号量 acquire 后、fetch 前加 `tokio::time::sleep(spider.download_delay()).await`，或文档说明 `download_delay` 在并发模式下被 per-domain 信号量替代。

### 3. per-domain 信号量许可数 = `max_concurrent`（语义待确认）

每个 domain 的 `Semaphore::new(max_concurrent)`，意味着同一个 domain 最多 `max_concurrent` 个并发。跨 domain 总并发也由 `buffer_unordered(max_concurrent)` 限制。这是 brief 的设计，已按 brief 实现，但 reviewer 需确认这是否符合预期（是否需要 per-domain 独立于全局 max_concurrent 的单独配置）。

### 4. `Cargo.lock` 有未提交改动（非本次引入）

`git status` 显示 `Cargo.lock` 被修改，但本次未改 `Cargo.toml`。这些改动可能是前序 task 遗留或 cargo 自动更新。按 brief 约束「不要修改 brief 之外的文件」，本次提交未包含 `Cargo.lock`。

### 5. 测试文件 `Arc` 未使用 warning

`tests/crawl_concurrency_test.rs` 第 3 行 `use std::sync::Arc;` 未被使用。这是 brief 原文代码，保持不变。`cargo check --tests` 有对应 warning，不影响编译。

## next BASE

```
7b431d53faf97edac0367b7dd1d54bb0953531c5
```

---

## Fix Round 1

针对 reviewer 在 `task-8-review.md` 中提出的 4 个 findings 进行修复。

### 修复的 findings 清单

- **C1 (Critical)**: follow channel 丢消息 — 在 `run()` 内新增 `Arc<AtomicUsize> in_flight` 计数器跟踪 in-flight future，unfold 终止条件改为「budget reached && in_flight==0」或「sched empty && in_flight==0」；future 内用 RAII `InFlightGuard` 确保所有退出路径递减计数器。
- **I1 (Important)**: `download_delay` 丢失 — 在 per-domain 信号量 `acquire_owned()` 后、`fetch_page(...)` 前加 `tokio::time::sleep(spider_clone.download_delay()).await`，恢复礼貌爬取延迟语义。
- **I2 (Important)**: `robots_cache` 锁跨 await — 在 robots check 代码块前加注释标注已知性能限制（全局 Mutex 在网络拉取期间被持有，序列化所有域 robots 检查），阶段 1 接受，不扩大范围重构 `RobotsCache`。
- **M5 (Minor)**: 测试未使用 import — 删除 `tests/crawl_concurrency_test.rs:3` 的 `use std::sync::Arc;`。

### C1 实现细节

**in-flight 计数器位置**: 在 `run()` 函数体内、`stream` 构造之前声明：
```rust
let in_flight = Arc::new(AtomicUsize::new(0));
```
通过两层 clone 进入 unfold closure（外层 `let stream = { ... }` 块捕获 + 内层 `stream::unfold` closure 捕获），再在构造 future 前 clone 一份 `in_flight_c` move 进 future。

**递增点**: unfold async block 内 pop 成功后、构造 future 前：
```rust
in_flight.fetch_add(1, Ordering::SeqCst);
```

**递减点（RAII guard）**: 在 `fetch_page` 函数前定义 module 私有 struct：
```rust
struct InFlightGuard {
    counter: Arc<AtomicUsize>,
}
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}
```
future 内首行创建 guard：`let _guard = InFlightGuard { counter: in_flight_c };`。guard 持有 `Arc` clone（而非 `&AtomicUsize` 引用），避免 async 状态机自引用导致的 Pin 问题。无论 future 从哪个 `return` 退出（domain filter skip / robots disallow / is_blocked / 正常完成 / fetch error），guard 的 `Drop` 都会执行 `fetch_sub(1, SeqCst)`。

**unfold 终止逻辑重写**: 原 `sched.pop().await?` 改为 `loop { drain channel; check budget; match sched.pop().await { ... } }`：
- budget 达上限：若 `in_flight == 0` 返回 `None`；否则 `yield_now` + `continue`（不再 pop 新请求，等 in-flight 完成）
- `sched.pop()` 返回 `None`：若 `in_flight == 0` 返回 `None`；否则 `yield_now` + `continue`（等 in-flight 发出 follow）
- `sched.pop()` 返回 `Some(req)`：`in_flight.fetch_add(1)`，构造 future，`return Some((fut, ()))`

### 测试结果

- `cargo check`: exit 0，编译通过。5 个 warnings 全部是预先存在的（browser/scraper/page/challenge/storage 模块），`src/crawl/mod.rs` 自身无 warning。
- `cargo test --lib`: exit 0，`test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`。所有 34 个 lib 单元测试通过，未破坏现有功能。
- `cargo check --tests`: exit 0，集成测试文件编译通过。`crawl_concurrency_test.rs` 的 unused import warning 已消失（M5 修复），仅剩 `adaptive_test.rs:58` 的预先存在 `unused variable: store` warning。

### 提交 hash

```
0b4e731202082d7ff0255b1d52bb5009ed6569f0
```

### 新增 concerns

- **C1 yield_now 轮询开销**: 当 sched 空且 in-flight > 0 时，unfold 用 `tokio::task::yield_now().await + continue` 重试。这是 cooperative 调度，不会 busy-spin，但若 in-flight future 长时间不完成（如网络超时），unfold 会被反复 schedule。生产场景下可接受（超时通常有上限），但若需更激进优化可改为 `tokio::time::sleep(Duration::from_millis(1))`。当前实现优先保证正确性，未做此优化。
- **C1 budget 达上限后的行为**: budget 达上限时不再 pop 新请求，但已 in-flight 的 future 仍会执行 `fetch_page` 并递增 `stats_pages`，最终 `pages_crawled` 可能略超 `max_pages`（最多 `max_concurrent - 1` 个，对应原 review 的 M2）。这是并发常见权衡，未在本次修复中处理（M2 标记为可不处理）。
- **I2 仅注释未重构**: 按任务约束「不扩大范围重构 RobotsCache」，I2 只加了注释标注性能限制。真实多域爬虫场景下 robots 检查会被序列化，建议后续 task 改 per-domain 锁或双检模式。
- **未跑 `cargo test --test crawl_concurrency_test`**: 遵循任务约束（该测试 `#[ignore]` 且需网络访问 httpbin.org），未执行。C1 的并发正确性仍无离线测试覆盖（对应原 review 的 M4），建议后续 task 用 wiremock 或本地 HTTP server 补离线测试。

### Fix Round 1 next BASE

```
0b4e731202082d7ff0255b1d52bb5009ed6569f0
```

reviewer 可用 `git diff 7b431d5..0b4e731` 生成完整 Fix Round 1 diff。
