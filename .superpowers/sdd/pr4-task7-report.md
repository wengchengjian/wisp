# PR4 Task 7 报告：re-export EngineConfig 到 wisp::crawl

## 任务概述

在 `src/crawl/mod.rs` re-export `runner::EngineConfig`，让用户可通过 `wisp::crawl::EngineConfig` 公开访问 PR4 Task 1-3 聚合后的引擎配置结构。

## 实现

### 改动 1：re-export（src/crawl/mod.rs:30）

```rust
// 原：
pub use runner::{Engine, EngineBuilder};
// 改为：
pub use runner::{Engine, EngineBuilder, EngineConfig};
```

### 改动 2：新增测试（src/crawl/mod.rs:486-492）

```rust
/// PR4 Task 7：验证 wisp::crawl::EngineConfig 公开可访问。
#[test]
fn test_engine_config_public_accessible() {
    let config = crate::crawl::EngineConfig::default();
    assert_eq!(config.max_concurrent, 8);
    assert_eq!(config.max_pages, 1000);
}
```

测试通过完整路径 `crate::crawl::EngineConfig::default()` 访问，验证 re-export 生效；并断言 `runner::EngineConfig::default()` 的 `max_concurrent=8, max_pages=1000`（与 brief 中预期一致，且与 `runner.rs:59-76` 的 `impl Default` 实际值匹配）。

## TDD 证据

### RED（实现前）

命令：`cargo test --lib test_engine_config_public_accessible`

```
error[E0433]: cannot find `EngineConfig` in `crawl`
   --> src/crawl/mod.rs:489:36
    |
489 |         let config = crate::crawl::EngineConfig::default();
    |                                    ^^^^^^^^^^^^ could not find `EngineConfig` in `crawl`
...
error: could not compile `wisp` (lib test) due to 1 previous error; 2 warnings emitted
```

退出码非 0，编译失败，符合预期（`EngineConfig` 尚未 re-export）。

### GREEN（实现后）

命令：`cargo test --lib test_engine_config_public_accessible`

```
running 1 test
test crawl::tests::test_engine_config_public_accessible ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

测试通过。

## 全量回归

命令：`cargo test --lib`

```
test result: ok. 278 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.85s
```

278 个 lib 测试全部通过（HEAD 之前 277 + 本任务新增 1），无回归。

命令：`cargo build --lib`

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.41s
```

编译成功（仅有 2 个预先存在的 `unused variable: fetcher` warning，位于 `src/fetcher/mod.rs:495,503`，与本次改动无关）。

## 文件变更

- `src/crawl/mod.rs`（+9 / -1 行）

## 自审清单

- **Completeness**：re-export 已添加，测试已添加并通过。✅
- **Quality**：改动最小（仅一行 re-export + 9 行测试），与 brief 完全一致。✅
- **Discipline (YAGNI)**：未添加任何未要求的 `pub use`、文档注释或顶层 re-export（brief Step 1 的"如需"条件未触发，因为 `wisp::crawl::EngineConfig` 已可通过现有路径访问，无需修改 `src/lib.rs`）。✅
- **Testing**：遵循 TDD（先 RED 后 GREEN），测试输出干净。✅
- **精准修改**：仅修改 `src/crawl/mod.rs`，未触碰其他文件。✅

## 提交

```
05a921e refactor: re-export EngineConfig 到 wisp::crawl
```

## 备注

- 预先存在的 2 个 warning（`src/fetcher/mod.rs:495,503` 的 `unused variable: fetcher`）不属于本任务范围，未处理。
- 未修改 `src/lib.rs`：brief 中"如需"为可选项，而 `wisp::crawl::EngineConfig` 已是公开访问路径，无需额外顶层 re-export。
