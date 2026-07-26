# Task 7 Review

## Spec compliance
1. ✅ `Cargo.toml:43`: `turso = { version = "=0.7.0-pre.18", optional = true }`，`rusqlite` 已完全移除（`src/` 与 `Cargo.toml` 均无残留匹配）
2. ✅ `Cargo.toml:11`: `sqlite = ["dep:turso"]`，feature gate 名称为 `sqlite` 未变
3. ✅ `src/storage/sqlite.rs:22`: `pub async fn open(path: &Path) -> Result<Self>`
4. ✅ `src/storage/sqlite.rs:36`: `pub async fn open_in_memory() -> Result<Self>`
5. ✅ `init_schema` 运行 `PRAGMA journal_mode=WAL` (line 51) 与 `PRAGMA synchronous=NORMAL` (line 56)。实施者用 `conn.query("PRAGMA journal_mode=WAL", ())` + 循环消费 rows 替代 `execute_batch`，原因在代码注释 line 50 与报告 §5.2 中说明：turso 的 `execute_batch` 不能消费返回行，而 `PRAGMA journal_mode=WAL` 返回一行新 mode。这是合理的 turso-specific 适配。
6. ✅ `init_schema` 检测旧三表 `element_snapshots/crawl_checkpoints/response_cache` 并 `tracing::warn!` (lines 60-76)
7. ✅ `init_schema` 运行 `super::migrations::SCHEMA_V1` (line 78)，与 `src/storage/migrations.rs` 中 SCHEMA_V1 一致（单表 kv 结构）
8. ✅ `Store::set` 用 `INSERT OR REPLACE INTO kv ... VALUES (?1, ?2, ?3, NULL, ?4)` + `turso::params![namespace, key, value.to_vec(), now]` (lines 91-94)
9. ✅ `Store::get` TTL 检查 `ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER)` (lines 104-107)
10. ✅ `Store::get` 对 `TursoValue::Blob(b)` 返回 `Some(b)`，对 `Null`/无行返回 `None` (lines 110-121)
11. ✅ `Store::delete` 执行 `DELETE FROM kv WHERE namespace = ?1 AND key = ?2`（实际删除，硬约束满足）(lines 127-130)
12. ✅ `Store::set_with_ttl` 设置 `ttl_secs` 与 `cached_at` (lines 138-145)
13. ✅ `src/bin/wisp.rs:125`: `Arc::new(wisp::SqliteStore::open(std::path::Path::new(&db)).await?)`
14. ✅ `src/storage/sqlite.rs` 中无 `spawn_blocking`（仅在 line 4 注释中提及）
15. ✅ 无 `parking_lot::Mutex` / `tokio::sync::Mutex` 包裹 Connection；`SqliteStore` 直接持有 `Database`

## Code quality
16. ✅ 错误映射统一为 `WispError::Storage(StorageError::General(...))`。已验证 `src/error.rs:169-173` 中 `StorageError::General(String)` 变体存在，且 `WispError::Storage(#[from] StorageError)` 在 line 209
17. ✅ 生产代码无 `unwrap()`；所有 26 处 `unwrap()` 均在 `#[cfg(test)] mod tests` 内（lines 163-251），符合项目规则
18. ✅ 无 backward-compat shim，无 `#[deprecated]` 标记
19. ✅ 无未使用导入：`Path`/`Duration`/`async_trait`/`turso::{Builder, Database, Value as TursoValue}`/`Result`/`StorageError`/`WispError`/`Store` 均在生成代码中引用。`use std::sync::Arc;` 仅在 `#[cfg(test)] mod tests` (line 153) 内导入，用于 `Arc<AtomicU32>` 跨 task 计数器，是测试必需（与 brief §7 提示一致）
20. ✅ 中文注释保留且与既有风格一致（`//! SQLite 存储后端（基于 turso，原生 async）。单表 KV 结构。`、`/// 打开或创建数据库文件。`、`// 旧 schema 检测`、`// PRAGMA journal_mode=WAL 返回一行...`）
21. ✅ PRAGMA WAL 偏差在报告 §5.2 中明确文档化，是 turso API 限制的合理适配

## Test quality
22. ✅ 所有 6 个测试均为 `#[tokio::test] async fn`（lines 160, 169, 183, 190, 199, 229）
23. ✅ `make_store()` 为 `async fn` (line 156)
24. ✅ 既有 5 个测试保留：`checkpoint_roundtrip`、`ttl_expiry`、`ttl_none_never_expires`、`namespace_isolation`、`old_schema_detection_does_not_break_new_store`
25. ✅ 新测试 `test_sqlite_store_async_does_not_block_runtime` (lines 229-256) 验证 counter > 10 且 write_elapsed < 5s
26. ✅ 报告 §3 显示 sqlite 模块 6/6 PASS，全量 435 passed / 0 failed / 64 ignored

## Verdict
APPROVED

## Findings (if any)
- [Minor] `clippy::cast_possible_wrap` 警告（`d.as_secs() as i64`，sqlite.rs:139）— 报告 §6 已确认原 rusqlite 版本同位置已有此警告，非本次回归。如需修复可后续统一用 `i64::try_from(d.as_secs()).unwrap_or(i64::MAX)` 或 `cast_signed()`，但属于无关清理，不应阻塞本次合并。
- [Minor] turso 为 pre-release 版本 `=0.7.0-pre.18`，API 可能后续变动 — 报告 §6 已提示需在 turso 正式版发布后跟踪迁移。
