# 阶段 1 Cleanup Report

**Commit**: `27f134f` — `chore: 阶段 1 cleanup（消除 warning + 修正注释 + 改进文档）`
**Base**: `7e33640`（HEAD before cleanup）
**日期**: 2026-07-21
**性质**: 零功能变更，仅消除 warning + 修正注释 + 改进文档

---

## 一、7 项 Cleanup 处理清单

| # | 位置 | 是否完成 | 改动内容 |
|---|---|---|---|
| 1 | `tests/crawl_checkpoint_test.rs:3` | ✅ | 删除未使用的 `use std::sync::Arc;`（消除 1 个 unused import warning） |
| 2 | `src/storage/mod.rs:45-48` | ✅ | 删除 `Store::conn()` dead code 方法 + 注释（消除 1 个 dead_code warning）。grep 确认全库无调用 |
| 3 | `src/crawl/scheduler.rs:86-94, 105-124` | ✅ | `seen_urls()`: 重写 doc 注释明确标注 stage 1 placeholder 语义（返回 hash 字符串非真实 URL），实现简化为一行；`restore()`: 加 `#[allow(dead_code)]` + doc 注释说明 stage 1 未启用、stage 2 启用 |
| 4 | `src/crawl/mod.rs:35-37` | ✅ | 修正 `#[serde(skip)]` 注释事实错误：原注释称"JSON 序列化语义不受影响"是错的，改为说明 `#[serde(skip)]` 对所有 Serializer 生效（含 serde_json） |
| 5 | `src/crawl/mod.rs:410-416` | ✅ | 定期 checkpoint 保存失败路径：`if let Ok(blob)` + `let _ =` 改为 `match` + 两个 `tracing::warn!`（序列化失败 / 保存失败），与已有的 `delete_checkpoint` 失败 `warn!` 保持一致 |
| 6 | `src/crawl/mod.rs:346` | ✅ | `sems.entry(domain.clone())` → `sems.entry(domain)`，移动所有权（确认 `domain` 在此行后未再使用） |
| 7 | `src/storage/mod.rs:14-15` | ✅ | Store 文档更新：明确"单 task 内访问无需 Mutex；多 task 并发访问需 `Arc<Mutex<Store>>`"，举例 Engine::run vs Spider::parse 内 adaptive save_element |

**总改动量**: 5 files changed, 222 insertions(+), 22 deletions(-)
（注：insertions 含 Cargo.lock 的 191 行新增，源码实际改动 ~30 行，符合 review 预估）

---

## 二、测试结果

### `cargo check --tests`
- **exit code**: 0
- **结果**: 编译通过
- **本 cleanup 消除的 warning（3 个）**:
  - ✅ `tests/crawl_checkpoint_test.rs:3` unused import `std::sync::Arc`
  - ✅ `src/storage/mod.rs` `Store::conn()` dead_code
  - ✅ `src/crawl/scheduler.rs` `Scheduler::restore()` dead_code（通过 `#[allow(dead_code)]` 消除）
- **剩余 warning（5 个，均为预先存在，非本分支引入）**:
  - `src/browser/mod.rs:55` unused import `CommandExt`
  - `src/scraper/mod.rs:185` unused variable `opts`
  - `src/page/mod.rs:17` field `headless` never read
  - `src/challenge/mod.rs:126,147` methods `wait_js_challenge`/`wait_managed` never used
  - `tests/adaptive_test.rs:58` unused variable `store`（不在本次 cleanup 范围）

### `cargo test --lib --test crawl_checkpoint_test --test difflib_test --test adaptive_test`
- **exit code**: 0
- **结果摘要**:
  - `lib`: 34 passed; 0 failed; 0 ignored
  - `crawl_checkpoint_test`: 4 passed; 0 failed; 0 ignored
  - `difflib_test`: 7 passed; 0 failed; 0 ignored
  - `adaptive_test`: 5 passed; 0 failed; 0 ignored
  - **总计 50 passed，0 failed**，与 review 预期一致

### 未运行的测试（按 task 约束跳过）
- `crawl_concurrency_test` — `#[ignore]` + 需网络
- `integration` — 需 Chrome

---

## 三、提交信息

- **Commit hash**: `27f134f`
- **Branch**: `master`
- **Commit message**: `chore: 阶段 1 cleanup（消除 warning + 修正注释 + 改进文档）`
- **Changed files**:
  - `Cargo.lock`（构建产物，随 cargo check 更新）
  - `src/crawl/mod.rs`
  - `src/crawl/scheduler.rs`
  - `src/storage/mod.rs`
  - `tests/crawl_checkpoint_test.rs`

---

## 四、未处理项

无。7 项 cleanup 全部完成，无阻塞项。

### Stage 2 需关注的遗留项（来自 final-review.md，本次不处理）
1. `Store` 共享模式（I2）— 若 stage 2 adaptive 集成 Engine，必须解决 `Arc<Mutex<Store>>`
2. `Scheduler::seen_urls` 真实化 — stage 2 应维护 `HashSet<String>` 原始 URL
3. `Scheduler::restore` 启用 — stage 2 改用 `restore()` 替代 `push()` 循环
4. `ElementSnapshot::capture` 重写 — stage 2 用 `Node::ancestors()/parent()` 替代 outer_html 重复解析
5. 离线并发测试 — stage 2/3 用 wiremock 补 Engine run 的并发正确性测试
6. `tests/adaptive_test.rs:58` unused variable `store` — 不在本次 cleanup 范围，可后续单独处理
