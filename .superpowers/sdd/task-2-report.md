# Task 2 Report — P1-1a status_codes 改用 DashMap 无锁计数

## Steps Taken (TDD: Red → Green → Commit)

### Step 1: 写失败测试
新建 `/home/weng/wisp/tests/p1_status_codes_test.rs`（逐字照搬 brief），2 个 `#[tokio::test]`：
- `status_codes_concurrent_increment_is_correct` — 50 个 task 各对 200/404 累加 100 次，断言各 5000
- `status_codes_snapshot_returns_empty_for_fresh_stats` — 新建 stats 快照为空

### Step 2: 验证失败（Red）
```
error[E0425]: cannot find function `record_status` in module `wisp::crawl`
error[E0603]: struct `SpiderStats` is private
```
符合预期：`record_status` 是 `pub(crate)` 不可见；`SpiderStats` 在 `mod.rs:50` 是 `use self::stats::SpiderStats;`（私有 use），外部不可达。

### Steps 3–9: 实现

**Step 3 — `src/crawl/observability/stats.rs`**
- imports：`tokio::sync::Mutex` → `dashmap::DashMap`
- 字段：`pub status_codes: Mutex<HashMap<u16, usize>>` → `pub status_codes: DashMap<u16, AtomicUsize>`
- 构造：`Mutex::new(HashMap::new())` → `DashMap::new()`
- 新增 `pub fn status_codes_snapshot(&self) -> HashMap<u16, usize>`（`iter().map(|r| (*r.key(), r.value().load(SeqCst))).collect()`）

**Step 4 — `src/crawl/engine.rs` `record_status`**
- `async fn` → 同步 `pub fn`（直接定为 `pub`，合并 Step 4 与 Step 8 的可见性改动）
- 内部：`stats.status_codes.entry(status).and_modify(|c| c.fetch_add(1, Relaxed)).or_insert(AtomicUsize::new(1))`

**Step 5 — 移除 4 处 `.await`**
`grep -n "record_status.*\.await" src/crawl/engine.rs` 定位 4 行（166/185/301/445），逐个 Edit 删 `.await`。完成后 grep 复核：0 匹配。

**Step 6 — `src/crawl/engine.rs:417`**
`stats.status_codes.lock().await.clone()` → `stats.status_codes_snapshot()`

**Step 7 — `src/crawl/runner.rs:390`**
`ctx.state.stats.status_codes.lock().await.clone()` → `ctx.state.stats.status_codes_snapshot()`

**Step 8 — `src/crawl/mod.rs` 暴露 API**
- 在 `pub use runner::{Engine, EngineBuilder};` 后追加 `pub use engine::record_status;`
- 额外：将 `mod.rs:50` 的 `use self::stats::SpiderStats;` 改为 `pub use self::stats::SpiderStats;` —— brief Step 8 假定 `SpiderStats` 已通过 `pub use observability::stats;` 可见，但该 re-export 仅暴露模块（`wisp::crawl::stats::SpiderStats` 可用），短路径 `wisp::crawl::SpiderStats` 实际不可见（Step 2 报错即因此）。改为 `pub use` 后测试导入通过。此为 brief 措辞与现状的小偏差，最小修复。

**Step 9 — 删除孤立死文件 `src/crawl/stats.rs`**
- `git rm src/crawl/stats.rs` 失败：`fatal: pathspec 'src/crawl/stats.rs' did not match any files`
- 原因：该文件未被 git 追踪（`git status` 显示 `?? src/crawl/stats.rs`，`git ls-files` 为空）
- 处置：改用文件系统删除（DeleteFile 工具）。文件本就 untracked，删除无需 git 暂存，提交时自然不包含。结果一致：仓库与工作树均不再有此死文件。

### Step 10: 验证通过（Green）
见下方"Verification Commands"。

### Step 11: 提交
```
git add src/crawl/observability/stats.rs src/crawl/engine.rs src/crawl/runner.rs src/crawl/mod.rs tests/p1_status_codes_test.rs
git commit -m "perf: status_codes 改用 DashMap 无锁计数 (P1-1a)"
```
未使用 `git add -A`/`git add .`。`src/crawl/stats.rs` 删除未单独暂存（文件 untracked，无删除差异可暂存）。

---

## Verification Commands & Output

### `cargo test --test p1_status_codes_test` — 期望 2 passed
```
running 2 tests
test status_codes_snapshot_returns_empty_for_fresh_stats ... ok
test status_codes_concurrent_increment_is_correct ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
✅ PASS

### `cargo build` — 期望无错误
```
warning: `wisp` (lib) generated 6 warnings (run `cargo fix --lib -p wisp` to apply 5 suggestions)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s
```
✅ 编译成功。6 个 rustc warnings 全部是 pre-existing unused-import（`Client`、`StreamExt` 等，baseline 即有），与本次改动无关。

### `cargo test --lib` — 期望 207 passed
```
test result: ok. 207 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
```
✅ 与 baseline 一致（207）。

### `cargo clippy --lib 2>&1 | grep "generated.*warnings"` — 期望 28 baseline
```
warning: `wisp` (lib) generated 27 warnings (run `cargo clippy --fix --lib -p wisp -- ` to apply 15 suggestions)
```
✅ 27 ≤ 28，无新增 warning（实际减少 1 个：移除 stats.rs 的 `tokio::sync::Mutex` import 后该处不再产生相关 lint）。

### 附加验证（相关集成测试）
```
cargo test --test crawl_checkpoint_test --test engine_infra_test --test multi_spider_test --test builder_api_test
test result: ok. 10 passed; 0 failed
test result: ok. 5 passed; 0 failed
test result: ok. 5 passed; 0 failed
test result: ok. 3 passed; 0 failed
```
✅ 无回归。

---

## Warnings Encountered & Resolution

1. **`git rm src/crawl/stats.rs` 失败**：文件未被 git 追踪（untracked）。改为文件系统删除。无影响：untracked 文件删除无需暂存，commit 后仓库与工作树均无该文件。已在报告 Step 9 说明。
2. **测试导入 `wisp::crawl::SpiderStats` 报 private**：brief Step 8 假定 `pub use observability::stats;` 已让 `SpiderStats` 短路径可见，实际仅暴露模块。最小修复：将 `mod.rs:50` `use` 改为 `pub use`。已在报告 Step 8 说明。
3. **clippy warnings 28 → 27**：减少而非增加，符合"无新增 warning"要求。无需处理。

---

## Final Commit

- SHA: `324b2a96061e20ed5f4cd56017caaac27dea226d`
- 短 SHA: `324b2a9`
- 分支: `master`
- 消息: `perf: status_codes 改用 DashMap 无锁计数 (P1-1a)`（一行，符合 brief 要求）
- 暂存文件（5）：
  - `src/crawl/observability/stats.rs` (M)
  - `src/crawl/engine.rs` (M)
  - `src/crawl/runner.rs` (M)
  - `src/crawl/mod.rs` (M)
  - `tests/p1_status_codes_test.rs` (A)
- 未暂存但已处理：`src/crawl/stats.rs` 删除（untracked，无删除差异）

---

## Self-Review Note

- 改动严格限定在 brief 列出的文件与接口内，未触碰范围外代码。
- `record_status` 直接定为 `pub`（合并 Step 4 的 `pub(crate)` 与 Step 8 的 `pub` 升级），避免重复编辑；语义与最终目标一致。
- 4 处 `.await` 移除逐个 Edit（每处上下文不同），grep 复核 0 残留。
- `status_codes_snapshot()` 实现与 brief 给出的代码逐字一致。
- `SpiderStats` 可见性修复（`use` → `pub use`）是 brief 措辞偏差的最小修复，非越权改动——测试用例逐字来自 brief，必须让其通过。
- 孤立文件删除方式调整（untracked → 文件系统删除），结果与 brief 目标一致。
- 未引入新 clippy warning，lib 测试数与 baseline 一致。
