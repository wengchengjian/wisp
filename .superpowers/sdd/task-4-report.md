# Task 4 Report — P1-2 Scheduler seen/heap 锁分离

## Steps Taken

1. **Step 1 — 写失败测试**：新建 `tests/p1_scheduler_test.rs`，逐字从 brief 复制两个测试（`scheduler_concurrent_push_pop_dedup_correct` 与 `scheduler_fingerprint_strategy_seen_split_works`）。
2. **Step 2 — 基线测试**：`cargo test --test p1_scheduler_test` → 2 passed（原 Mutex 实现也通过，记录基线 0.00s）。
3. **Step 3 — 重构 imports + struct 定义**：
   - imports 增加 `use dashmap::DashSet;`
   - 删除 `SchedulerInner`，新增 `HeapInner { heap, seq }`
   - `Scheduler` 改为 `heap: Arc<Mutex<HeapInner>>` + `seen_exact: Arc<DashSet<String>>` + `seen_fp: Arc<DashSet<u64>>` + `strategy: DedupStrategy`
4. **Step 4 — `with_strategy`**：替换为分别构造 `HeapInner` 与两个 `DashSet` 的版本。
5. **Step 5 — `push`**：先 DashSet insert 查重，命中才锁 heap 入队。
6. **Step 6 — `pop`**：锁改为 `self.heap`。
7. **Step 7 — `pending_urls`**：锁改为 `self.heap`。
8. **Step 8 — `seen_urls`**：直接对 DashSet iter 收集，不锁 heap。
9. **Step 9 — `len`/`is_empty`**：锁改为 `self.heap`。
10. **Step 10 — `restore`**：清 seen DashSet → 锁 heap 清空 → 重建 seen（Fingerprint 模式 `parse::<u64>()` 回填）→ 锁 heap 重排队 pending。
11. **Step 11 — 全量验证**：见下。
12. **Step 12 — 提交**：`git add src/crawl/scheduling/scheduler.rs tests/p1_scheduler_test.rs && git commit -m "perf: Scheduler seen/heap 锁分离 (P1-2)"`，仅暂存这 2 个文件。

## Verification Commands & Output

```
$ cargo test --test p1_scheduler_test
running 2 tests
test scheduler_fingerprint_strategy_seen_split_works ... ok
test scheduler_concurrent_push_pop_dedup_correct ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo test --lib crawl::scheduling
running 5 tests
test crawl::scheduling::cron::tests::test_parse_every_30_min ... ok
test crawl::scheduling::cron::tests::test_parse_valid_cron ... ok
test crawl::scheduling::scheduler::tests::fingerprint_seen_roundtrip_preserves_hashes ... ok
test crawl::scheduling::cron::tests::test_next_run_is_in_future_or_none ... ok
test crawl::scheduling::cron::tests::test_parse_invalid_cron ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 202 filtered out; finished in 0.00s

$ cargo test --lib
test result: ok. 207 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s

$ cargo build
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.65s

$ cargo clippy --lib 2>&1 | grep "generated.*warnings"
warning: `wisp` (lib) generated 27 warnings (run `cargo clippy --fix --lib -p wisp -- ` to apply 15 suggestions)
```

所有命令均退出码 0。clippy 27 warnings = 基线，未引入新警告。

## Warnings Encountered & Resolution

无新增 clippy 警告。`src/crawl/mod.rs` 既有 6 个 unused import 警告为既有基线（`Client` / `StreamExt` / `Mutex` 等），与本次改动无关，未触碰。`HashSet` 仍被 `seen_urls()` 返回类型与 `restore()` 参数类型使用，未变为未用。`fingerprint()` 函数仍被 `push` 与 `restore` 使用。无偏离 brief 之处。

## Final Commit

- **SHA**: `2524f3add7f197c0482b3a178b3a82543ade7538`
- **Branch**: `master`
- **Message**: `perf: Scheduler seen/heap 锁分离 (P1-2)`
- **Staged files**（仅 2 个，符合 brief）:
  - `src/crawl/scheduling/scheduler.rs`（82 行变更：39 删 43 增）
  - `tests/p1_scheduler_test.rs`（新增 44 行）

## Self-Review Note

- 严格按 brief 12 步执行，所有代码块逐字采用，无自由发挥。
- `SchedulerInner` 完全移除；剩余 `g.heap` / `g.seq` 引用均作用于 `HeapInner`（合法）。`self.inner`、`g.strategy`、`g.seen_exact`、`g.seen_fp`、`SchedulerInner` 在文件中已 0 处残留（grep 确认）。
- `DedupStrategy` 因 `Copy` 直接作为字段（非 `Arc`），与 brief 指引一致。
- `#[derive(Clone)]` 仍合法：`Arc<Mutex<HeapInner>>`、`Arc<DashSet<_>>`、`DedupStrategy`（Copy）均 Clone。
- `restore` 在 Fingerprint 模式下使用 `url.parse::<u64>()` 回填 `seen_fp`，未对已指纹字符串再次 `fingerprint()`，符合 brief 关键提示。既有单测 `fingerprint_seen_roundtrip_preserves_hashes` 仍通过。
- 范围未越界：仅改 `scheduler.rs` + 新建 `tests/p1_scheduler_test.rs`，未改动 Engine / Spider / 其他模块。
