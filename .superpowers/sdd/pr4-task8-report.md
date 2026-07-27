# PR4 Task 8 报告：最终集成验证 + banzhu-rs 兼容性检查

## 实施内容

在 `src/crawl/runner.rs` 的 `tests` 模块末尾追加集成验证测试 `test_engine_full_lifecycle_with_config_accessor`：

- 实现 `OkSpider`（impl Spider，空 start_urls，handle 返回空）
- 通过 `Engine::infra()` 构建 Engine，调用 6 个 setter（max_concurrent/max_pages/max_errors/fetch_mode/obey_robots/download_delay_ms）
- 通过 `engine.config()` 访问器验证 6 个 config 字段值正确
- 调用 `engine.run_stream(OkSpider).events()` 验证收到 `CrawlEvent::Done` 事件

### 与 brief 的偏离

brief 给出的测试代码未在测试函数内导入 `Duration`，但 `mod tests` 不继承父模块的 `use`，直接使用 `Duration::from_millis(10)` 会编译失败。按现有测试 `test_checkpoint_spawned_not_blocking_main_loop` 的模式，在测试函数内增加 `use std::time::Duration;`。其余代码与 brief 一致。

## 验证结果

### wisp（本仓库）

| 命令 | 结果 |
|---|---|
| `cargo build --lib` | ✅ 编译成功（2.68s），无新增警告 |
| `cargo test --lib test_engine_full_lifecycle_with_config_accessor` | ✅ 1 passed; 0 failed |
| `cargo test --lib` | ✅ 279 passed; 0 failed; 0 ignored |
| `cargo test --lib --features stealth` | ✅ 338 passed; 0 failed; 12 ignored |

预先存在的警告（与本任务无关）：
- `src/fetcher/mod.rs:495/503` unused variable `fetcher`（2 处）

### banzhu-rs 兼容性

| 命令 | 结果 |
|---|---|
| `cd /home/weng/banzhu-rs && cargo build` | ✅ 编译成功（23.21s） |
| `cd /home/weng/banzhu-rs && cargo test --lib` | ⚠️ 62 passed; 12 failed |

### banzhu-rs 12 个失败的分析

失败测试列表：
- `crypto::tests::test_decrypt_section_simple`（`src/crypto.rs:137` — `assertion failed: result.is_some()`）
- `db::tests::test_fts_search_basic`（`src/db/mod.rs:523` — `assertion failed: !results.is_empty()`）
- `db::tests::test_fts_search_by_author`
- `db::tests::test_fts_search_by_content`
- `db::tests::test_fts_search_by_title`
- `db::tests::test_fts_search_count`
- `db::tests::test_fts_search_long_content`
- `db::tests::test_fts_search_phrase`
- `db::tests::test_fts_search_prefix`
- `db::tests::test_fts_search_relevance_score`
- `db::tests::test_fts_search_result_fields`
- `db::tests::test_fts_search_special_chars`

**与 PR4 无关**。已通过 `git stash` 验证：在 wisp HEAD 05a921e（本任务变更前）banzhu-rs 同样有 12 个失败（62 passed / 12 failed），完全一致。失败位于 banzhu-rs 自己的 `crypto.rs`（解密）和 `db/mod.rs`（FTS 全文搜索）模块，与 wisp 的 Engine/EngineConfig/EngineShared 重构无关。本任务变更仅 46 行测试代码追加，不影响任何生产代码。

## Files Changed

- `src/crawl/runner.rs`：+46 行（仅测试函数追加）

## Commits

- `661739a` — test: PR4 集成验证 Engine 配置聚合完整生命周期

## 自检清单

- **完整性：** 所有验证步骤（build/test/stealth/banzhu-rs build/banzhu-rs test）均已运行
- **质量：** 集成测试覆盖 config() 访问器 + run_stream 完整生命周期，验证 PR4 核心目标（Engine 配置聚合）
- **纪律：** 仅追加测试，无生产代码改动，无过度工程
- **测试：** wisp 全部 279 个 lib 测试通过；stealth feature 组合 338 个测试通过；banzhu-rs 编译成功

## 关切事项

banzhu-rs 有 12 个预先存在的测试失败（crypto/FTS search），与本任务无关，已在变更前后通过 stash 验证一致性。按任务约束"如需修复，请报告但不要修复"，仅报告不修复。
