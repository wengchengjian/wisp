# Task 10: 更新现有测试与文档

## 目标

1. 跑全量编译 `cargo build` 和全量测试 `cargo test`
2. 修复任何因本次重构（Task 1-9）导致的测试编译错误
3. 更新文档（如有 CLAUDE.md）

## Files

- Modify: `tests/integration.rs`（如有 SpiderResponse 构造，补 from_cache: false）
- Modify: `tests/fetch_test.rs`（同上）
- Modify: `CLAUDE.md`（如有，说明新 patterns/until 用法）
- 其他因重构导致的编译失败的测试文件

## 主要问题点（预期）

1. `SpiderResponse` 构造缺 `from_cache` 字段（Task 4 引入）
   - 注意：Task 4 已修复大部分，但可能有遗漏的构造点
2. `EngineContext` 字段变化导致测试中直接调用 `run_spider_once` 的点失败
   - 注意：`run_spider_once` 已在 Task 8 中删除，改为 `run_with_sender`
3. 其他因 Spider trait 变化（patterns/matches/until）或 EngineBuilder 变化导致的编译错误

## 预先存在的问题（不要修复）

以下文件有预先存在的 GBK 编码问题（非本次重构引入），不要尝试修复，在报告中说明即可：
- `tests/real_scrape_test.rs`
- `tests/cf_bypass_real_test.rs`
- `tests/session_test.rs`

这些文件在重构前就无法编译，用 `git stash` 验证过非本次引入。如果 cargo test 因这些文件失败，可以用 `--lib` 和指定测试文件的方式验证本次重构的测试。

## 验证步骤

1. `cargo build` — 必须通过（lib + bins）
2. `cargo test --lib` — 必须通过
3. `cargo test --test stop_condition_test` — 必须通过
4. `cargo test --test builder_api_test` — 必须通过
5. `cargo test --test multi_spider_test` — 必须通过
6. 对于其他 tests/*.rs：
   - 如能编译则跑
   - 如因预先存在的 GBK 编码问题失败，在报告中说明
   - 如因本次重构导致编译错误，修复

## 文档更新

如有 `CLAUDE.md`，补充说明：
- `Spider::patterns(&self) -> Vec<String>` — URL 路由匹配模式（正则字符串）
- `Spider::matches(&self, url: &str) -> bool` — 默认实现，编译 patterns 为 regex
- `Spider::until(&self) -> Arc<dyn StopCondition>` — per-spider 终止策略
- `SpiderBuilder::patterns(Vec<String>)` — 设置路由模式
- `SpiderBuilder::until<C: StopCondition>(cond: C)` — 设置终止条件
- `Engine::spiders(Vec<Arc<dyn Spider>>)` — 多 Spider 共享队列

如无 CLAUDE.md，跳过文档更新。

## Commit

PowerShell 兼容，用 -m：
```powershell
git add tests/ CLAUDE.md
git commit -m "test: 适配共享队列重构与 from_cache 字段" -m "修复因 Task 1-9 重构导致的测试编译错误"
```
（如无 CLAUDE.md 改动，只 add tests/）
