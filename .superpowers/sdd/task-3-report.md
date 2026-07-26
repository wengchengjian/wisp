# Task 3 报告：follow_rx Mutex 移除（H5）

## 状态

DONE

## Commit

- Hash: `b1d5c88`
- Message: `perf(runner): follow_rx 移入 unfold 状态移除 Mutex`
- 父提交: `a50f2f7`（Task 1+2 完成）

## 测试结果摘要

- `cargo test --all-features --no-fail-fast`: **426 passed, 5 failed (pre-existing), 64 ignored**
- 目标测试 `crawl::runner::tests::test_follow_rx_drained_without_mutex`: **PASS**
- `cargo build --all-features`: **编译通过，无新 warning**
- `cargo clippy --all-targets --all-features`: engine.rs / runner.rs **无新 warning**

### Pre-existing 失败（与本次改动无关，已通过 stash 验证）

1. `auto_mode_test::test_generalize_uuid` — auto 模式 URL 泛化逻辑
2. `auto_mode_test::test_generalize_mixed` — auto 模式 URL 泛化逻辑
3. doctest `src/crawl/middleware/mod.rs:11` — `follow_with` 类型不匹配
4. doctest `src/crawl/builder.rs:7` — `follow_with` 类型不匹配
5. doctest `src/crawl/builder.rs:27` — `follow_with` 类型不匹配

## 修改文件列表

### 1. `src/crawl/engine.rs`

**关键改动**：
- 删除 `EngineShared` 结构体的 `follow_rx: Arc<Mutex<UnboundedReceiver<Request>>>` 字段，替换为 3 行注释说明设计原因（Receiver 单消费者无需 Mutex）
- 删除 4 处测试构造函数中的 `follow_rx` 初始化：
  - `make_ctx()` — 基础测试 ctx
  - `make_ctx_with_retry(max_retries)` — 带 RetryMiddleware
  - `make_ctx_auto(max_retries)` — Auto 模式
  - `make_ctx_with_tx(max_retries)` — 带事件通道
- 每处 `let (follow_tx, follow_rx) = mpsc::unbounded_channel::<Request>();` 改为 `let (follow_tx, _) = ...`（测试不需要单独持有 receiver）
- `use tokio::sync::Mutex;` 保留（rule_engine/items/cf_domain_locks 仍用 Mutex）

### 2. `src/crawl/runner.rs`

**关键改动**：
- `run_inner` 中 `EngineShared` 构造删除 `follow_rx: Arc::new(Mutex::new(follow_rx)),` 行
- unfold 状态从 `()` 改为 `(Arc<EngineContext>, UnboundedReceiver<Request>)`
- unfold 闭包签名从 `move |_|` 改为 `move |(ctx, mut rx)|`
- 删除外层闭包内 `let ctx = ctx.clone();`（ctx 直接从状态元组取出）
- 删除 drain 逻辑的 Mutex 包装：
  ```rust
  // 旧：
  let mut rx_guard = ctx.shared.follow_rx.lock().await;
  while let Ok(req) = rx_guard.try_recv() { ... }
  drop(rx_guard);

  // 新：
  while let Ok(req) = rx.try_recv() { ... }
  ```
- unfold 返回值从 `Some((fut, ()))` 改为 `Some((fut, (ctx, rx)))`
- 文件末尾新增 `#[cfg(test)] mod tests`，含 `test_follow_rx_drained_without_mutex` 测试

### 3. `src/crawl/builder.rs`

- **无修改**（grep 确认无 follow_rx 引用）

## 编译错误与解决方式

### 1. 无编译错误

本次改动一次编译通过，无错误。核心设计点：

- unfold 状态元组 `(ctx, follow_rx)` 中的 `ctx` 是 `Arc<EngineContext>`，从状态取出后通过 `let ctx_c = ctx.clone();` 克隆一份给 fut，原 `ctx` 仍可 move 回状态元组返回
- `follow_rx` 是 `UnboundedReceiver<Request>`，move 进 unfold 闭包后每次循环通过状态传递，符合单消费者语义
- 外层 `ctx` 在 unfold 启动后仍要用于 checkpoint/pipeline close/final stats，所以 unfold 内部用的是 `ctx.clone()`（原代码已有此模式）

### 2. Clippy 对比

通过 stash 对比 baseline (a50f2f7) 与 with_changes 的 clippy 输出：
- baseline: 639 warnings
- with_changes: 638 warnings（减少 1 个）
- engine.rs/runner.rs 相关 warning 仅 `this function has too many lines` 行数从 274 减到 270（删除 4 行 follow_rx 代码导致，非新 warning）
- 删除的 warning 类型：`matching over () is more explicit`（unfold 状态从 `()` 改为元组后自然消失）

## 自审结果

### 设计正确性

1. **Receiver 单消费者语义**：`tokio::sync::mpsc::UnboundedReceiver` 实现是单消费者的，本身串行化所有 `try_recv` 调用，原 `Arc<Mutex<UnboundedReceiver>>` 的 Mutex 是冗余的。本次改动符合语义。
2. **unfold 状态传递**：每次循环返回 `Some((fut, (ctx, rx)))`，状态 `(ctx, rx)` 被传递给下次循环。fut 内部用 `ctx.clone()` 持有独立的 Arc，与状态中的 ctx 互不影响。
3. **buffer_unordered 兼容**：unfold 产出 `Stream<Item = Future>`，buffer_unordered 并发执行这些 Future。状态元组在每次循环结束时返回，不参与并发，符合 unfold 语义。

### 性能改进

1. **删除 Mutex 锁争用**：原实现每次 drain follow_rx 都要 `lock().await`，并发请求多时产生锁争用。新实现直接 `try_recv`，无锁。
2. **删除无谓 await**：原 `lock().await` 是 await point，即使 channel 空也要切换任务。新实现 `try_recv` 是同步非阻塞调用。
3. **删除 drop(rx_guard)**：原实现显式 drop guard 释放锁，新实现无需此操作。

### 风险评估

1. **并发安全**：UnboundedReceiver 不是 Clone，只能有一个消费者。unfold 是单线程驱动的（每次循环一个状态），不会有并发访问 rx 的情况。✓
2. **向后兼容**：本次改动不向后兼容（按 CLAUDE.md 要求），EngineShared 字段删除会导致所有使用旧字段的代码编译失败。已确认 engine.rs/runner.rs/builder.rs 中所有引用已清理。✓
3. **测试覆盖**：新增 `test_follow_rx_drained_without_mutex` 测试验证 try_recv drain 模式可行；全量回归 426 passed（含原有 runner/engine 测试），5 pre-existing failures 与本次改动无关。✓

### 约束符合性

- ✅ 不向后兼容
- ✅ 现有测试全过（pre-existing 失败除外，已 stash 验证）
- ✅ 修改文件无新 clippy warning（baseline 639 → with_changes 638）
- ✅ commit message 中文简短一行
- ✅ 工作目录 /home/weng/wisp

## Concerns

无。
