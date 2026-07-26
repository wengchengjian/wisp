# Task 5 报告：checkpoint spawn 后台 + EventBus 并发 listener

## 1. 状态

**DONE_WITH_CONCERNS**

主要功能（EventBus 并发 emit + checkpoint spawn 后台）已完整实现并通过 TDD 验证。
存在 1 个非阻塞性的设计 concern（见第 8 节），以及 2 个 pre-existing 失败测试（与 Task 5 无关）。

## 2. 提交的 commit hash

```
43de9d3 perf(observability): EventBus 并发 listener + checkpoint spawn 后台
```

父 commit：`1429817` (Task 4)

## 3. 测试结果摘要

| 测试范围 | 结果 |
| --- | --- |
| `crawl::observability::events::tests`（4 tests，含新增 1） | ✅ 4 passed, 0 failed |
| `crawl::runner::tests`（2 tests，含新增 1） | ✅ 2 passed, 0 failed |
| Lib 全量（301 tests） | ✅ 293 passed, 0 failed, 8 ignored |
| Integration (5 tests) | ✅ 5 passed, 0 failed |
| `auto_mode_test`（13 tests） | ⚠️ 11 passed, **2 failed（pre-existing）** |

**Pre-existing 失败**：`test_generalize_uuid`、`test_generalize_mixed`（URL 泛化正则 bug）
- 已通过 `git stash && cargo test --test auto_mode_test` 验证在 master HEAD `1429817` 同样失败
- 与 Task 5 改动无关（涉及 `auto` 模块的 URL pattern generalization）

## 4. 修改的文件列表和关键改动

### `src/crawl/observability/events.rs`（+133/-15）

1. **新增 import**：`use futures::stream::{FuturesUnordered, StreamExt};`

2. **`EventBus::emit` 改并发实现**（第 122-137 行）：
   - 旧实现：`for listener in &self.listeners { listener(event.clone()).await; }` 串行 await
   - 新实现：用 `FuturesUnordered` 收集所有 listener future 后并发 await，总延迟从 `sum(单 listener)` 降为 `max(单 listener)`
   - 添加 OPTIMIZE 注释说明设计意图

3. **新增测试 `test_event_bus_concurrent_listeners`**（第 320-374 行）：
   - 3 个均 50ms 的 listener
   - 串行 sum=150ms（FAIL > 80ms 阈值），并发 max=50ms（PASS < 80ms 阈值）
   - 验证 TDD red→green 流程

### `src/crawl/runner.rs`（+60/-25）

1. **`run_inner` 中 checkpoint 调用改 `tokio::spawn` 后台**（第 470-492 行）：
   - 旧实现：直接 `engine::persist_spider_checkpoint(...).await` 阻塞主循环
   - 新实现：clone `store/spider_name/sched/stats` 后 `tokio::spawn(async move { ... })`
   - 主循环立即继续处理下一请求，checkpoint 在后台异步执行
   - 失败仅 `tracing::warn!`（按 brief 设计，**见第 8 节 concern**）

2. **新增测试 `test_checkpoint_spawned_not_blocking_main_loop`**（第 678-703 行）：
   - 验证 `tokio::spawn` 不阻塞当前 task 的语义契约

### `src/crawl/engine.rs`（**无改动**）

`persist_spider_checkpoint` 签名保持不变：
```rust
pub(crate) async fn persist_spider_checkpoint(
    store: &dyn crate::storage::Store,
    spider_name: &str,
    sched: &scheduler::Scheduler,
    stats: &Arc<SpiderStats>,
) -> Result<()>
```
- 调用方通过 `Arc::clone` + Deref coercion 适配，签名无需调整

## 5. EventBus API 实际签名（与 brief 差异）

| Brief 假设 | 实际 API | 处理方式 |
| --- | --- | --- |
| `bus.on(move \|_event: &EngineEvent\| { ... })` | `bus.on(listener: EventListener)` 接收 `Arc<dyn Fn(EngineEvent) -> BoxFuture<'static, ()> + Send + Sync>` | 测试改为 `bus.on(Arc::new(move \|_event: EngineEvent\| { ... }))`，closure 接收 owned `EngineEvent` |
| `EngineEvent::Started { url }` | `EngineEvent::CrawlStarted { spider, start_urls }` | 测试改用 `CrawlStarted` |
| Listener 接收 `&EngineEvent` | Listener 接收 owned `EngineEvent` | 无需调整 clone 策略，原 `emit` 已用 `listener(event.clone())` |

## 6. persist_spider_checkpoint 签名是否调整

**未调整**。原签名 `(&dyn Store, &str, &Scheduler, &Arc<SpiderStats>)` 在 spawn 调用模式下完全可用：
- `Arc::clone(store).as_ref()` → `&dyn Store` ✓
- `&spider_name` (String) → `&str` via Deref ✓
- `&sched` (`&Arc<Scheduler>`) → `&Scheduler` via Deref coercion ✓
- `&stats` (`&Arc<SpiderStats>`) → `&Arc<SpiderStats>` 直接匹配 ✓

## 7. 遇到的编译错误及解决方式

### 错误 1：测试用 `(50ms + 5ms + 5ms)` 串行总和 60ms 仍 < 80ms 阈值，TDD red 阶段未失败

**现象**：按 brief 原始测试代码（一个慢 50ms + 两个快 5ms），旧串行实现下总耗时约 60ms，
仍满足 `< 80ms` 断言，测试 PASS。TDD red 阶段失效。

**原因**：brief 设计的 listener 耗时组合（50+5+5=60）与阈值（80）之间没有正确的不等式关系：
TDD 要求 `concurrent_max < threshold < serial_sum`，即 `50 < 80 < 60`，但 80 > 60 不成立。

**解决**：改为 3 个均 50ms 的 listener，使 serial_sum=150、concurrent_max=50、阈值 80 严格区分：
`50 < 80 < 150`，TDD red→green 流程正常。

### 错误 2：测试模块 `Arc` 未在作用域

**现象**：`error[E0433]: cannot find type Arc in this scope` 在 `runner.rs` 测试模块。

**原因**：runner.rs 的 `mod tests` 没有 `use super::*;`，父模块的 `use std::sync::Arc;` 不会自动 glob 导入到子模块。

**解决**：在测试函数内部显式 `use std::sync::Arc;`。

### 错误 3：`uninlined_format_args` clippy 警告

**现象**：测试中 `assert!(..., "实际 {:?}", elapsed)` 触发 `clippy::uninlined_format_args`。

**解决**：改为内联格式 `assert!(..., "实际 {elapsed:?}")`。
最终 clippy 输出与 master HEAD 完全一致（638 warnings, 0 errors），**无新增 clippy 警告**。

## 8. 自审结果

### Concern 1（设计权衡，非阻塞）：checkpoint 失败不再发送 `CrawlEvent::Error`

**原行为**（ND-003-ERR 设计）：checkpoint 失败时通过 `tx.try_send(CrawlEvent::Error { ... })` 通知 stream 消费者，避免静默吞掉。

**新行为**（按 brief）：仅 `tracing::warn!`，不再发送 `CrawlEvent::Error`。

**影响**：stream 消费者（如 `run_stream` 用户）无法再感知 checkpoint 失败。
但 `tracing::warn!` 仍记录到日志，运维侧可观测性保留。

**为何按 brief 执行**：brief 注释明确写 "OPTIMIZE: spawn 后台执行，主循环不等待；失败仅 tracing::warn"。
"不向后兼容，只考虑最优解" 的设计哲学也支持简化失败路径。
若需保留事件可见性，可将 `tx` 也 clone 进 spawn（`try_send` 对 closed channel 安全返回错误），
但 brief 未要求，未实现。

### Concern 2（pre-existing）：2 个失败测试 `test_generalize_uuid` / `test_generalize_mixed`

与 Task 5 完全无关，涉及 `auto` 模块的 URL pattern generalization 逻辑。
在 master HEAD `1429817` 同样失败，已确认 pre-existing。

### TDD 验证

- ✅ Step 2 验证旧串行实现测试 FAIL（elapsed 150ms > 80ms）
- ✅ Step 4 验证新并发实现测试 PASS（elapsed 50ms < 80ms）
- ✅ Step 6 验证 spawn 契约测试 PASS

### Clippy 验证

- master HEAD: 638 warnings, 0 errors
- 本 commit: 638 warnings, 0 errors
- **新增 clippy 警告：0**（满足"修改文件无新 clippy warning"约束）

### Build 验证

- `cargo build --all-features` 成功
- `cargo test --lib --all-features` 293 passed, 0 failed

## 9. 文件清单

修改文件（已提交）：
- `/home/weng/wisp/src/crawl/observability/events.rs`
- `/home/weng/wisp/src/crawl/runner.rs`

未修改文件（brief 列出但实际无需改动）：
- `/home/weng/wisp/src/crawl/engine.rs`
