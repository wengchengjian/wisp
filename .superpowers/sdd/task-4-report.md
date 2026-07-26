# Task 4 报告：fetch_dispatch 退避抖动 + rule_engine 单次锁（M1 + M4）

## 1. 状态

**DONE**

## 2. 提交信息

- **Commit hash**: `14298175baa5e5055aad1f8953ad0165b6382973`
- **Commit message**: `perf(engine): fetch_dispatch 退避抖动 + rule_engine 单次锁`
- **分支**: master（直接开发，HEAD 推进 1 个 commit）

## 3. 测试结果摘要

| 测试范围 | 结果 |
| --- | --- |
| 新增 3 个测试（exponential_backoff / jitter_range / single_lock_for_autofallback） | 3/3 通过 |
| 新增 1 个测试（retry_middleware_propagates_when_retries_exhausted） | 1/1 通过 |
| engine + middleware 模块全量测试 | 46/46 通过 |
| lib 单元测试（全量） | 291/291 通过，8 ignored |
| bin 集成测试 | 5/5 通过 |
| `auto_mode_test` 集成测试 | 11/13 通过（2 个 pre-existing 失败，详见下文） |
| Clippy（归一化对比 baseline） | 52 = 52，**无新增 warning** |

**Pre-existing 失败确认**：`tests/auto_mode_test.rs::test_generalize_uuid` 和 `test_generalize_mixed`。已用 `git stash` 在 HEAD `b1d5c88`（本任务改动前）复现，确认这两个失败与 Task 4 无关（URL 泛化逻辑测试）。

## 4. 修改的文件列表和关键改动

### `src/crawl/engine.rs`
- **导入**：新增 `use rand::RngExt;`（rand 0.10 起 `random_range` 需要 trait 显式导入）。
- **Step 8 — AutoFallback 单次锁**：把 `fetch_dispatch` 中原本两次 `rule_engine.lock().await`（一次 `resolve`、一次 `learn`）合并为单次锁，锁内完成 `resolve` + `learn`。条件反转以避免 `clippy::if_not_else`：`if resolve == Some(Stealth) { false } else { learn; true }`。
- **Step 9 — Retry 路径指数退避 + 抖动**：在 `ErrorAction::Retry` 分支 `continue` 前新增退避逻辑：
  - `exp_delay = base_ms * 2^attempt`，封顶 30s（`attempt` 上限 10 防溢出）
  - `jitter = rand::rng().random_range(0..exp_delay/2 + 1)`
  - `tokio::time::sleep(exp_delay + jitter).await`
  - `tracing::debug!` 增加 `(backoff {:?})` 字段
- **`#[allow(clippy::too_many_lines)]`**：标注 `fetch_dispatch`（107 行，超 100），核心分发函数职责本身复杂，brief 未要求拆分。
- **Step 2/4/6 测试**：追加 3 个测试到 `tests` 模块（在 `fetch_dispatch_no_duplicate_autofallback_for_learned_url` 后）：
  - `test_retry_exponential_backoff`：纯函数验证退避算法（200/400/800/25600/30000/30000）
  - `test_retry_jitter_range`：纯函数验证抖动上界（101/201/15001）
  - `test_rule_engine_single_lock_for_autofallback`：用 `MockRuleEngine` 演示单次锁调用模式（同步函数，避免 `unused_async` 警告）
- **RetryMiddleware::new 调用点**：3 处测试辅助函数（`make_ctx_with_retry` / `make_ctx_auto` / `make_ctx_with_tx`）的 `RetryMiddleware::new(std::time::Duration::ZERO)` 改为 `RetryMiddleware::new(max_retries)`，与 `EngineConfig.max_retries` 保持一致。

### `src/crawl/middleware/builtin.rs`
- **Step 10a — RetryMiddleware 结构体**：`retry_delay: Duration` → `max_retries: u32`；`new(retry_delay: Duration)` → `new(max_retries: u32)`。文档注释更新为"退避由 engine 统一负责"。
- **Step 10b — process_error**：删除 `tokio::time::sleep(self.retry_delay).await`；改为 `if req.retry_count < self.max_retries { Retry } else { Propagate }`（中间件层双重保险，与 engine 的 `max_retries` 检查互不干扰）。
- **DefaultMiddlewareConfig**：新增 `pub max_retries: u32` 字段。
- **default_middlewares**：`RetryMiddleware::new(Duration::from_millis(500))` → `RetryMiddleware::new(cfg.max_retries)`。
- **测试更新**：
  - `test_retry_middleware_always_retries_fetch_errors`：`RetryMiddleware::new(Duration::ZERO)` → `RetryMiddleware::new(3)`，注释更新。
  - 新增 `test_retry_middleware_propagates_when_retries_exhausted`：验证 `retry_count >= max_retries` 时返回 `Propagate`。
  - 3 处 `DefaultMiddlewareConfig` 测试用例（`default_middlewares_classifies_by_mode_and_config`）补充 `max_retries: 3` 字段。

### `src/crawl/middleware/mod.rs`
- **Step 11 — doc 示例**：`RetryMiddleware::new(3, std::time::Duration::from_secs(1))` → `RetryMiddleware::new(3)`（修复原本就是双参数的过时示例）。

### `src/crawl/runner.rs`
- `DefaultMiddlewareConfig` 构造点（`SpiderRunner::run_inner`）补充 `max_retries: self.max_retries` 字段。

## 5. rand API 实际使用方式

- **Cargo.toml**：`rand = "0.10"`（实际解析到 `0.10.2`）
- **调用方式**：`rand::rng().random_range(0..exp_delay / 2 + 1)`
- **关键差异**：rand 0.10 起 `random_range` 方法定义在 `RngExt` trait 上，必须显式 `use rand::RngExt;` 才能调用（不像 0.8/0.9 自动在 prelude）。brief 中提到的 `try_from_rng` / `SysRng` / `SmallRng` 写法在 0.10 仍可用，但简洁起见采用 `rand::rng()` + `random_range`。

## 6. 遇到的编译错误及解决方式

| 错误 | 原因 | 解决 |
| --- | --- | --- |
| `no method named random_range found for ThreadRng` | rand 0.10 的 `random_range` 在 `RngExt` trait 中，需显式导入 | `use rand::RngExt;` |
| `mismatched types: expected u32, found Duration`（3 处 engine.rs 测试） | `RetryMiddleware::new` 签名从 `Duration` 改为 `u32` 后，测试调用点仍是 `Duration::ZERO` | 改为 `RetryMiddleware::new(max_retries)` |
| `unused async for function with no await statements`（MockRuleEngine::resolve_and_learn） | brief Step 6 测试用 `async fn` 但内部无 `await` | 改为同步 `fn resolve_and_learn(&self) -> bool`，调用去掉 `.await` |
| `clippy::if_not_else`（engine.rs:411） | `if x != Some(...) { ...; true } else { false }` 触发 | 反转条件：`if x == Some(...) { false } else { ...; true }` |
| `clippy::too_many_lines`（fetch_dispatch 107/100） | 退避抖动逻辑让函数超过 100 行 | 加 `#[allow(clippy::too_many_lines)]` 标注（brief 未要求拆分核心函数） |

## 7. 自审结果

### Brief 步骤覆盖
- [x] Step 1：rand 依赖检查（`rand = "0.10"`，已适配 API）
- [x] Step 2/4/6：3 个失败测试已写
- [x] Step 3/5/7：3 个测试通过
- [x] Step 8：AutoFallback 单次锁实现（合并 resolve+learn 到单次 `lock().await`）
- [x] Step 9：Retry 路径指数退避 + 抖动实现（rand 0.10 API）
- [x] Step 10：RetryMiddleware 移除 `retry_delay` + `sleep`，改为 `max_retries`
- [x] Step 11：mod.rs doc 示例更新
- [x] Step 12：所有 `RetryMiddleware::new` 调用点更新（builtin.rs × 1 + engine.rs × 3 测试 + mod.rs × 1 doc）
- [x] Step 13：engine + middleware 测试通过
- [x] Step 14：全量回归（仅 pre-existing 失败）
- [x] Step 15：clippy 无新增 warning
- [x] Step 16：commit 提交

### 关键设计决策
1. **RetryMiddleware::new(max_retries: u32)**：brief 明确要求改为 `max_retries` 参数。这使中间件层与 engine 层各自独立检查 `retry_count < max_retries`，构成双重保险（避免单点逻辑漂移）。`DefaultMiddlewareConfig` 因此新增 `max_retries` 字段，由 `runner.rs` 从 `SpiderRunner.max_retries` 传入，保证两端一致。
2. **AutoFallback 单次锁**：语义与原实现等价（已学习 Stealth 的 URL 不重复 learn），但避免两次锁争用。`should_upgrade` 用块作用域限定锁的生命周期，`continue` 在锁释放后执行。
3. **退避抖动**：`exp_delay + jitter`，抖动范围 `[0, exp_delay/2]`（半区间，避免与退避同步）。`attempt.min(10)` 防止 `1u64 << attempt` 在大 attempt 时溢出。
4. **brief 参考代码与实际差异**：
   - brief 中 `process_error` 签名是 `(&self, req, _error)`，实际 trait 是 `(&self, _req, _err, _ctx)`，保留实际签名。
   - brief 中 AutoFallback 用 `!= Some(Stealth)`，实际改为 `== Some(Stealth)` 反转以避免 clippy 警告。
   - brief Step 6 测试用 `async fn resolve_and_learn`，实际改为同步函数以避免 `unused_async` 警告。

### 验证清单
- [x] 编译通过（lib + tests + bins）
- [x] 新增 4 个测试全部通过
- [x] 现有测试无回归（46/46 + 291/291 + 5/5）
- [x] Clippy 无新增 warning（归一化对比 baseline 52 = 52）
- [x] 2 个 pre-existing 失败已用 git stash 验证非本任务引入
- [x] 提交 message 用中文简短一行
