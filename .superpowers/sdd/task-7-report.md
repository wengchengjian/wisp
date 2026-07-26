# Task 7 (Round 2): Turso 替换 rusqlite — 报告

## 1. Status

DONE

## 2. Commit

`04978fa` — `perf(storage): turso 替换 rusqlite，原生 async 无需 spawn_blocking`

## 3. Test Results

| 命令 | 结果 |
| --- | --- |
| `cargo build --all-features` | ✅ 编译通过（9.92s） |
| `cargo test --lib storage::sqlite --features sqlite` | ✅ 6 passed / 0 failed |
| `cargo test --all-features` | ✅ 435 passed / 0 failed / 64 ignored |
| `cargo clippy --all-targets --all-features` | ✅ 编译通过，无新增警告 |

sqlite 模块测试明细：
- `checkpoint_roundtrip` ... ok
- `ttl_expiry` ... ok
- `ttl_none_never_expires` ... ok
- `namespace_isolation` ... ok
- `old_schema_detection_does_not_break_new_store` ... ok
- `test_sqlite_store_async_does_not_block_runtime` ... ok

## 4. Files Modified

- `Cargo.toml` — `rusqlite = "0.39"` 替换为 `turso = { version = "=0.7.0-pre.18", optional = true }`；feature gate `sqlite = ["dep:rusqlite"]` → `sqlite = ["dep:turso"]`
- `Cargo.lock` — 自动更新（rusqlite 移除，turso 及依赖添加）
- `src/storage/sqlite.rs` — 完整重写为 turso async API：
  - `SqliteStore::open` / `open_in_memory` 改为 `async fn`
  - 移除 `Arc<parking_lot::Mutex<Connection>>` 包裹，直接持有 `Database`（turso 内部管理连接池）
  - 所有 `Store` 方法直接 `.await`，不再 `spawn_blocking`
  - `init_schema` 保留旧 schema 检测（三表 → 单表 kv）
- `src/bin/wisp.rs:125` — `SqliteStore::open(...)` 调用添加 `.await`

## 5. Deviations from Brief

1. **Cargo.toml 原版 rusqlite 版本**：brief 提到 `rusqlite = "0.31"`，实际原仓库为 `rusqlite = "0.39"`。不影响结果，已正确替换为 turso。
2. **PRAGMA journal_mode=WAL 实现**：brief 使用 `conn.execute_batch("PRAGMA journal_mode=WAL;")`，但 turso 的 `execute_batch` 不能消费返回行（WAL PRAGMA 返回一行新 mode）。实际代码改用 `conn.query("PRAGMA journal_mode=WAL", ())` + 循环消费 rows。这是适配 turso 实际 API 的必要调整。
3. **`use std::sync::Arc;`**：brief 提示可能需要删除，但实际 tests 模块中 `test_sqlite_store_async_does_not_block_runtime` 测试需要 `Arc<AtomicU32>` 用于跨 task 计数器，因此在 tests 模块内保留 `use std::sync::Arc;`（模块内导入，非顶层）。这是合理的。

## 6. Concerns / Notes

- **clippy 警告 `cast_possible_wrap`（sqlite.rs:139）**：`d.as_secs() as i64` 转换可能 wrap。验证后确认原 rusqlite 版本同位置已有此警告（`git show HEAD:src/storage/sqlite.rs:143` 显示相同代码），非本次回归。如需修复可后续统一处理（使用 `cast_signed()`）。
- **turso 是 pre-release 版本**（`0.7.0-pre.18`）：API 可能后续变动，但当前功能完整、测试通过。需在 turso 正式版发布后跟踪迁移。
- **性能预期**：移除 `spawn_blocking` 后，SQLite 操作不再占用 blocking 线程池，`test_sqlite_store_async_does_not_block_runtime` 测试验证了后台 task 在写入期间继续运行（counter > 10）。
