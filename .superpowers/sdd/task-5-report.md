# Task 5: StorageError 细分（新增 4 变体）实施报告

## 实施摘要

按 plan `docs/superpowers/plans/2026-07-26-arch-refactor-pr1-cookiejar-storage.md` 第 1359-1562 行的 5 个步骤严格执行，新增 4 个 `StorageError` 变体（`NotFound` / `Serialization` / `Backend` / `Corrupted`）+ `Io(#[from] std::io::Error)`，并将 `storage/mod.rs` 的 4 个业务函数从 `General` 迁移到具体变体。

### 变更文件

| 文件 | 变更 |
| --- | --- |
| `src/error.rs` | `StorageError` enum 新增 5 变体；末尾追加 `#[cfg(test)] mod tests` 共 7 个测试 |
| `src/storage/mod.rs` | `save_element`/`load_element`/`save_response`/`load_response` 改用 `Serialization`/`Corrupted` |

### 5 个步骤执行记录

1. **Step 1（写失败测试）**：在 `src/error.rs` 末尾追加 `#[cfg(test)] mod tests`，含 7 个测试：
   - `storage_error_general_display`
   - `storage_error_not_found_display`
   - `storage_error_serialization_display`
   - `storage_error_backend_display`
   - `storage_error_corrupted_display`
   - `storage_error_io_from_std_io_error`
   - `storage_error_converts_to_wisp_error`

2. **Step 2（验证失败）**：`cargo test --lib error::tests` 编译失败，报 `no variant NotFound`、`no variant Corrupted`、`no variant Serialization`、`no variant Backend`、`From<std::io::Error> not implemented`，共 7 个错误，符合预期。

3. **Step 3（最小实现）**：替换 `src/error.rs:167-173` 的 `StorageError` enum，新增 `NotFound { namespace, key }` / `Serialization(String)` / `Backend(String)` / `Corrupted(String)` / `Io(#[from] std::io::Error)`，保留 `General(String)` 向后兼容。

4. **Step 4（验证通过）**：`cargo test --lib error::tests` → 7 passed; 0 failed。

5. **Step 5（迁移业务函数 + 提交）**：
   - `save_element` 序列化错误：`General` → `Serialization`
   - `load_element` 解析错误：`General` → `Corrupted`
   - `save_response` 序列化错误：`General` → `Serialization`
   - `load_response` 解析错误：`General` → `Corrupted`
   - `cargo test --lib storage::tests` → 6 passed; 0 failed
   - `git commit -m "feat: StorageError 新增 NotFound/Serialization/Backend/Corrupted 变体"`

## 测试结果

| 测试集 | 结果 |
| --- | --- |
| `cargo test --lib error::tests` | **7 passed; 0 failed** |
| `cargo test --lib storage::tests` | **6 passed; 0 failed** |
| `cargo test --lib`（全 lib 回归） | **315 passed; 0 failed; 10 ignored**（共 325，含新增 7） |
| `cargo build --all-features` | 成功（含 sqlite feature） |

## Commit

```
1afdd14 feat: StorageError 新增 NotFound/Serialization/Backend/Corrupted 变体
```

父提交：`decdb60`（Task 4 BrowserCookieJar）。

变更统计：`2 files changed, 88 insertions(+), 5 deletions(-)`。

## 自审发现

1. **未改 file.rs/sqlite.rs/engine.rs**：`src/storage/file.rs`（11 处）、`src/storage/sqlite.rs`（24 处）、`src/crawl/engine.rs:610,617`（2 处）仍使用 `StorageError::General`。Plan Step 5 显式只要求修改 `storage/mod.rs` 的 4 个业务函数（行 159-211），未要求改 file.rs/sqlite.rs/engine.rs。`General` 变体保留向后兼容，符合 plan 设计意图（"新代码应使用具体变体"）。后续 PR 可逐步迁移。

2. **`From<io::Error>` 双路径无歧义**：`WispError::Io(#[from] std::io::Error)` 与 `StorageError::Io(#[from] std::io::Error)` 同时存在。`?` 运算符根据函数返回类型解析：返回 `Result<_, StorageError>` 时走 `StorageError::Io`，返回 `Result<_, WispError>` 时走 `WispError::Io`。`cargo build --all-features` 成功证明无冲突，与 plan 风险评估（行 2114）一致。

3. **测试无 `unwrap`/`expect` 滥用**：7 个测试全部使用 `assert_eq!`/`assert!`/`matches!`，符合项目约定。

4. **命名/注释规范**：变量 snake_case ✓，注释中文 ✓，提交信息中文一行 ✓。

5. **接口契约**：`Produces: crate::error::StorageError::{NotFound, Serialization, Backend, Corrupted, Io}` —— 与 plan 行 1368 完全一致。

## 状态

✅ **完成**。Task 5 已合入 master（`1afdd14`）。所有测试通过，无回归。PR1 的 5 个 Task 全部完成。
