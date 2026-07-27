# PR4 Task 5 报告：删除 EngineShared，字段合并到 EngineContext 顶层

## 实现内容

按 task brief 要求完成 PR4 Task 5 重构：

1. **删除 `EngineShared` struct**：将 8 个字段（sched / follow_tx / proxy_clients / control / work_notify / middleware_chain / rule_engine / cf_domain_locks）直接展开到 `EngineContext` 顶层。

2. **全局替换 `ctx.shared.xxx` → `ctx.xxx`**：
   - `src/crawl/engine.rs`：17 处代码替换（涵盖 `check_control_and_hook`、`process_request`、`process_response`、`fetch_dispatch` 及 5 个测试中的访问）
   - `src/crawl/runner.rs`：12 处代码替换（涵盖 `run_inner` 中 EngineContext 构造后的所有访问，包括 InFlightGuard 创建中 `ctx_c.shared.work_notify` → `ctx_c.work_notify`，2 处）

3. **更新 4 个测试辅助函数**（make_ctx / make_ctx_with_retry / make_ctx_auto / make_ctx_with_tx）：将原 `shared: EngineShared { ... }` 包装层删除，字段直接展开到 EngineContext 顶层。

4. **新增测试 `test_engine_context_no_shared_substruct`**：验证 `ctx.sched` / `ctx.control` / `ctx.work_notify` / `ctx.middleware_chain` 字段可直接访问。

5. **保留 EngineState**（按约束，不删除）。

6. **EngineContext 文档更新**：说明 PR4 重构后字段分三组（只读配置 / 跨 task 共享可变 / per-run 可变）。

## 测试与验证

### TDD 证据

**RED 阶段**（实现前）：
```
$ cargo test --lib test_engine_context_no_shared_substruct
error[E0609]: no field `sched` on type `engine::EngineContext`
error[E0609]: no field `control` on type `engine::EngineContext`
error[E0609]: no field `work_notify` on type `engine::EngineContext`
error[E0609]: no field `middleware_chain` on type `engine::EngineContext`
error: could not compile `wisp` (lib test) due to 4 previous errors
```

**GREEN 阶段**（实现后）：
```
$ cargo test --lib test_engine_context_no_shared_substruct
test crawl::engine::tests::test_engine_context_no_shared_substruct ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 275 filtered out
```

### 完整验证

| 命令 | 结果 |
| --- | --- |
| `cargo test --lib test_engine_context_no_shared_substruct` | 1 passed; 0 failed |
| `cargo build --lib` | 编译成功 |
| `cargo test --lib` | 276 passed; 0 failed; 0 ignored |

Task 4 测试 `test_engine_context_config_is_arc_runner_config` 仍通过（未被破坏）。

## 文件改动

| 文件 | 改动 |
| --- | --- |
| `src/crawl/engine.rs` | 删除 EngineShared struct（48-67 行原）；EngineContext 字段直接展开；17 处 `ctx.shared.xxx` → `ctx.xxx`；4 个测试辅助函数更新；新增 `test_engine_context_no_shared_substruct` 测试 |
| `src/crawl/runner.rs` | run_inner 中 EngineContext 构造删除 `shared: EngineShared { ... }` 包装；12 处 `ctx.shared.xxx` → `ctx.xxx`（含 InFlightGuard 创建中 2 处 `ctx_c.shared.work_notify`） |

提交：`8859255 refactor: 删除 EngineShared，字段合并到 EngineContext 顶层`

净变更：+134 / -138 行。

## 自我审查

**Completeness（完整性）：**
- ✅ 8 个字段全部展开到顶层（grep 验证 EngineShared 仅剩 3 处文档注释引用）
- ✅ EngineState 按约束保留
- ✅ Task 4 测试未被破坏

**Quality（质量）：**
- ✅ 字段分组清晰，文档说明重构目的
- ✅ 沿用现有代码风格（注释格式、字段排序）
- ✅ 提交信息符合规范

**Discipline（纪律）：**
- ✅ 仅修改 task brief 指定的两个文件
- ✅ 未添加额外功能或重构
- ✅ 未触碰无关代码

**Testing（测试）：**
- ✅ TDD 流程严格遵循（RED → GREEN）
- ✅ 276 个 lib 测试全部通过
- ⚠️ 集成测试（`cargo test --tests`）失败，但确认是预先存在的问题（`mcp` feature 未启用、`ElementSnapshot::from_row` 重命名），与 PR4 Task 5 改动无关——通过 `git stash` 验证 HEAD 之前同样失败

## 关键发现与注意事项

1. **EngineContext 字段分组清晰**：删除 `EngineShared` 包装后，`EngineContext` 字段分为三组（只读配置 / 共享可变 / per-run 可变），通过注释分组标识，可读性提升。

2. **测试代码中保留历史引用注释**：`engine.rs:1640` 处的注释 `// 直接访问 ctx.sched（原 ctx.shared.sched）` 保留了对历史命名的引用，用于解释测试意图（验证字段直接访问而非通过子结构）。

3. **集成测试预先存在问题**：`cargo test --tests` 失败原因与 PR4 Task 5 无关，是 `mcp` feature 未启用 / `ElementSnapshot::from_row` 重命名导致的预先存在问题。

4. **EngineState 暂未删除**：按 task brief 约束保留，留待后续 task 处理。
