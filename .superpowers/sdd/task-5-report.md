# Task 5 Report — P1-7: SpiderRequest.meta 跨 checkpoint 持久化

## Steps Taken

1. **读取 brief 与当前 mod.rs**：确认 brief（`/home/weng/wisp/.superpowers/sdd/task-5-brief.md`）与 mod.rs 当前状态（line 50 为 `pub use self::stats::SpiderStats;`，line 88-96 为旧 Task 3 注释块 + `#[serde(skip)]`）。
2. **Step 1 — 写失败测试**：新建 `tests/p1_meta_persistence_test.rs`，逐字抄录 brief 中的两个测试（`meta_survives_bincode_roundtrip` 与 `meta_default_null_when_absent`）。
3. **Step 2 — 验证失败**：`cargo test --test p1_meta_persistence_test`，`meta_survives_bincode_roundtrip` 失败（`restored.meta` 为 `Null`），`meta_default_null_when_absent` 通过。符合预期。
4. **Step 3 — 插入 `meta_serde` 模块**：在 `src/crawl/mod.rs` line 50 后、`/// HTTP method for spider requests.` 注释前插入 brief 提供的私有 `meta_serde` 模块。
5. **Step 4 — 替换 serde 属性**：将旧 Task 3 注释块（7 行）+ `#[serde(skip)]` 替换为 brief 的新 3 行 P1-7 注释 + `#[serde(with = "meta_serde")]`，保留 `pub meta: Value,`。
6. **Step 5 — 验证通过**：`cargo test --test p1_meta_persistence_test && cargo test --lib`，2 个新测试 PASS，lib 207 全绿。
7. **Step 6 — checkpoint 集成测试**：`cargo test --test cr_fix_engine_test --test engine_infra_test`，8 个测试全绿；额外跑 `crawl_checkpoint_test`（5 个测试）全绿，clippy 27 warnings（与基线一致，无新增）。
8. **Step 7 — 提交**：`git add src/crawl/mod.rs tests/p1_meta_persistence_test.rs && git commit -m "feat: SpiderRequest.meta 跨 checkpoint 持久化 (P1-7)"`，仅暂存 2 个指定文件。

## Verification Commands & Output

```
$ cargo test --test p1_meta_persistence_test
running 2 tests
test meta_default_null_when_absent ... ok
test meta_survives_bincode_roundtrip ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test --lib | tail
test result: ok. 207 passed; 0 failed; 0 ignored; 0 measured

$ cargo test --test cr_fix_engine_test --test engine_infra_test
test test_stop_context_queue_size_is_real ... ok
test test_stopped_spider_url_not_silently_dropped ... ok
test test_fetch_retry_count_semantics ... ok
test result: ok. 3 passed; 0 failed (cr_fix_engine_test)
test test_engine_shutdown_via_control ... ok
test test_engine_control_isolation ... ok
test test_engine_run_returns_items ... ok
test test_engine_control_reset_between_runs ... ok
test test_engine_multiple_runs_share_resources ... ok
test result: ok. 5 passed; 0 failed (engine_infra_test)

$ cargo test --test crawl_checkpoint_test  # 额外验证 checkpoint 路径
test test_crawl_state_new_defaults ... ok
test test_checkpoint_load_missing_returns_none ... ok
test test_checkpoint_delete ... ok
test test_checkpoint_save_load_roundtrip ... ok
test checkpoint_restore_preserves_seen_urls ... ok
test result: ok. 5 passed; 0 failed

$ cargo clippy --lib 2>&1 | grep "generated.*warning"
warning: `wisp` (lib) generated 27 warnings

$ git commit -m "feat: SpiderRequest.meta 跨 checkpoint 持久化 (P1-7)"
[master 64ecb8f] feat: SpiderRequest.meta 跨 checkpoint 持久化 (P1-7)
 2 files changed, 54 insertions(+), 8 deletions(-)
```

## Warnings Encountered & Resolution

- **brief 代码编译错误（E0599）**：brief Step 3 提供的 `meta_serde` 模块 `use serde::{Deserializer, Serialize, Serializer};` 缺少 `Deserialize` trait，导致 `Vec::<u8>::deserialize(d)?` 报 `no associated function named deserialize found`。
  - **Resolution**：在 import 中补上 `Deserialize`，即 `use serde::{Deserialize, Deserializer, Serialize, Serializer};`。这是 brief 代码能编译通过的最小必要修复，未改变 brief 的任何逻辑或设计。
- **lib 测试数差异**：brief Step 5 预期 "lib 206 全绿"，实际 207。Project context 明确写 "lib 207 passed"，所以 207 是正确基线，brief 文档的 206 是旧值，无问题。

## Final Commit

- **SHA**: `64ecb8f16559c745d25c94f74838e56f6dab0d3b`
- **Branch**: `master`
- **Message**: `feat: SpiderRequest.meta 跨 checkpoint 持久化 (P1-7)`
- **Staged files** (2):
  - `src/crawl/mod.rs`
  - `tests/p1_meta_persistence_test.rs`
- **Diff stat**: 2 files changed, 54 insertions(+), 8 deletions(-)

## Self-Review Note

- **Scope**: 仅修改 brief 指定范围 — 新增 `meta_serde` 私有模块、将 `meta` 字段的 `#[serde(skip)]` 改为 `#[serde(with = "meta_serde")]`、新增 2 个测试。未触碰 `proxy` / `fetch_mode_override`（仍为 `#[serde(skip)]`），未做任何超出范围的改动。
- **No scope creep**: 没有重构周边代码，没有给未改动代码加注释/类型标注，没有删除 mod.rs 中既有的其它 unused import 警告（保持基线 27 warnings）。
- **Brief's exact code used**: 是。`meta_serde` 模块（含 `serialize`/`deserialize` 两函数、JSON 字节往返逻辑）、新注释文案、`#[serde(with = "meta_serde")]`、测试文件内容均逐字采用 brief 提供的版本。唯一偏离是上述必要的 `Deserialize` import 补全（brief 遗漏导致编译失败），属最小修复。
- **验证完整**: 失败测试 → 修复 → 通过 → lib 全绿 → checkpoint 集成测试全绿 → clippy 无新增 warning → 仅暂存 2 个指定文件提交。
