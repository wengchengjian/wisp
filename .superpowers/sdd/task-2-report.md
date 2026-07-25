# Task 2 实施报告：Engine 默认值 + bin/wisp.rs CLI + mcp 测试改动

## 状态
**DONE_WITH_CONCERNS** — brief 列出的全部修改完成，编译通过、mcp 单测 10/10 通过；但 brief Step 7 的两个端到端测试因 Task 1 留下的 API 破坏编译失败（不在 Task 2 范围内），见"顾虑"。

## 修改的文件（绝对路径）
- `/home/weng/wisp/src/crawl/runner.rs` — `Engine::infra()` 默认值：`cache_store`/`checkpoint_store` 从 `None` 改为注入 `MemoryStore::default()` / `FileStore::default()`
- `/home/weng/wisp/src/bin/wisp.rs` — `McpCmd::Serve` 分支：默认 `FileStore::default()`，`#[cfg(feature="sqlite")]` 分支保留 `--db` 走 `SqliteStore::open`
- `/home/weng/wisp/src/mcp/tools.rs` L276 — 测试用 `MemoryStore::default()` 替换 `SqliteStore::open_in_memory()`
- `/home/weng/wisp/src/mcp/mod.rs` L265 — 同上
- `/home/weng/wisp/src/lib.rs` L73 — re-export 追加 `FileStore`（brief 未列出但必须，否则 `bin/wisp.rs` 中 `wisp::FileStore` 无法解析）
- `/home/weng/wisp/.gitignore` — 追加 `wisp-data/`

## 关键决策
1. **lib.rs re-export 追加 FileStore**：按任务上下文要求执行，确保 `bin/wisp.rs` 中 `wisp::FileStore::default()` 可解析。已验证 `src/storage/mod.rs:15` 已 `pub use file::FileStore;`，`src/storage/file.rs:74` 已 `impl Default for FileStore`。
2. **未触碰端到端测试代码**：`tests/crawl_checkpoint_test.rs` 与 `tests/crawl_cache_real_test.rs` 编译失败是因为它们仍在调用 Task 1 已删除的 `SqliteStore::save_checkpoint/load_checkpoint/delete_checkpoint/load_response` 方法。brief Files 列表未包含这两个测试文件，按"精准修改"原则未改动。
3. **未加 `#[allow(unused_attributes)]`**：按任务上下文建议，先编译看实际效果。结果是 cargo 对 `#[cfg(feature = "sqlite")]` 产生 `unexpected_cfgs` 警告（2 个），符合预期，Task 3 添加 feature 定义后消失。
4. **git add 范围**：仅 `git add src/ .gitignore`，未包含 `.superpowers/sdd/` 或 `docs/` 的他人改动。

## 编译输出摘要
- 命令：`cd /home/weng/wisp && cargo build`
- 结果：**Finished exit 0**，0 错误
- 警告：
  - lib 293 个 `missing_docs` 历史警告（与本次改动无关）
  - bin "wisp" 2 个 `unexpected_cfgs` 警告（`feature = "sqlite"` 未在 Cargo.toml 定义，Task 3 修复）
  - 无 `unused variable: db` 警告（`let _ = db;` 生效）

## 测试结果摘要

### Step 6: `cargo test --lib mcp`
- 结果：**10 passed; 0 failed; 0 ignored**（260 filtered out）
- 用时：0.03s
- 通过测试列表：
  - mcp::tests::test_handle_initialize
  - mcp::tests::test_tools_list_has_five_tools
  - mcp::tests::test_handle_tools_call_unknown_tool
  - mcp::tools::tests::test_extract_css_missing_args
  - mcp::tools::tests::test_fetch_page_missing_url
  - mcp::tools::tests::test_adaptive_scrape_missing_args
  - mcp::tools::tests::test_extract_css_returns_attr
  - mcp::tools::tests::test_extract_css_returns_text
  - mcp::tools::tests::test_stealth_fetch_missing_url
  - mcp::tools::tests::test_crawl_site_missing_args

### Step 7: 端到端测试

#### `cargo test --test crawl_checkpoint_test`
- 结果：**编译失败**（9 个 E0599 错误 + 1 个 unused_imports 警告）
- 失败原因：测试代码调用 `SqliteStore::save_checkpoint / load_checkpoint / delete_checkpoint` 等方法，这些方法在 Task 1 已删除，改为 `crate::storage::save_checkpoint(store, ...)` 等自由函数。
- 失败位置：
  - tests/crawl_checkpoint_test.rs:22, 24, 46, 47, 49, 50, 56, 87, 90
- 不在 Task 2 brief 范围内（brief Files 列表未含此文件）。

#### `cargo test --test crawl_cache_real_test`
- 结果：**编译失败**（1 个 E0599 错误 + 1 个 unused_imports 警告）
- 失败原因：测试代码调用 `Arc<SqliteStore>::load_response`（L45），该方法在 Task 1 已删除。
- 测试本身标了 `#[ignore = "requires network access"]`，但编译失败导致无法跳过执行。
- 不在 Task 2 brief 范围内。

## Step 8: wisp-data 目录检查
- 命令：`ls -la wisp-data/`
- 观察：目录存在但为空（仅 `.`/`..`，无子目录）。与任务上下文预测一致——`Engine::infra().build()` 时 `FileStore::default()` 可能创建根目录，但因端到端测试编译失败未实际触发 checkpoint 写入，故无 `checkpoint/` `element/` `response/` 子目录。
- git 不跟踪空目录，故 `wisp-data/` 未出现在 `git status` untracked 中；仍在 `.gitignore` 中追加 `wisp-data/` 以防后续写入数据。

## 提交信息
- commit hash: `ffd38f3b114a541484781fa8f5429c022252a13f`（短：`ffd38f3`）
- message: `feat(engine): 默认注入 MemoryStore + FileStore + CLI 适配`
- 范围：6 files changed, 22 insertions(+), 10 deletions(-)
  - src/bin/wisp.rs, src/crawl/runner.rs, src/lib.rs, src/mcp/mod.rs, src/mcp/tools.rs, .gitignore

## 顾虑与疑问
1. **端到端测试 API 破坏未修复**：`tests/crawl_checkpoint_test.rs` 与 `tests/crawl_cache_real_test.rs` 因 Task 1 删除 `SqliteStore::save_checkpoint` 等方法而编译失败。brief Step 7 期望它们能跑通验证 Engine 默认值生效，但实际无法编译。建议后续任务（或单独 fixup）将这两个测试改用 `wisp::storage::{save_checkpoint, load_checkpoint, ...}` 自由函数 + `MemoryStore`/`FileStore`。这不在 Task 2 brief Files 列表内。
2. **brief 与现实的矛盾**：brief Step 7 Expected 写"验证 Engine 默认值生效，checkpoint 持久化到 ./wisp-data/"，但因测试编译失败无法验证。Engine 默认值本身的代码改动已通过编译验证，且 mcp 单测中的 `Engine::infra().build()` 也成功，逻辑上默认值生效；但端到端"checkpoint 持久化到 wisp-data/"未能通过测试实证。
3. **`#[cfg(feature="sqlite")]` 警告**：2 个 `unexpected_cfgs` 警告存在，Task 3 在 Cargo.toml 添加 `sqlite` feature 后会消失。非阻塞。
4. **`wisp-data/` 目录为空**：未被 git 跟踪（空目录），`.gitignore` 已加入 `wisp-data/` 防止后续数据污染。
