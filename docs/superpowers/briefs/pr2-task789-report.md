# PR2 Task 7+8+9 报告：browser/stealth 边界清理

## 任务状态

**DONE**

## 三个 Commit Hash

```
a2f4fba docs: 清理 Browser::launch 注释，移除 anti-detection 描述
32ace9a refactor: build_stealth_args 拆为 build_common_args + build_stealth_extra_args
80d4988 refactor: browser/patches.rs 迁移到 stealth/patches.rs
```

## 测试运行结果摘要

- **Task 7 验证**（`cargo test --lib stealth::patches && cargo test --lib browser::page`）：3 passed; 0 failed
- **Task 8 验证**（`cargo test --lib browser::launch`）：10 passed; 0 failed（4 个新测试 + 6 个现有测试）
- **Task 9 验证**（`cargo test --lib browser`）：32 passed; 0 failed; 7 ignored
- **最终验证**（`cargo build --lib && cargo test --lib`）：
  - `cargo build --lib`：无错误、无警告
  - `cargo test --lib`：337 passed; 0 failed; 12 ignored

## 执行细节

### Task 7: 文件迁移

- 创建 `src/stealth/patches.rs`（与原 `src/browser/patches.rs` 内容完全相同）
- 删除 `src/browser/patches.rs`
- `src/browser/mod.rs`：删除 `pub mod patches;` 及其上方注释 `/// 反检测 JS 补丁注入。`
- `src/stealth/mod.rs`：按字母序在 `human` 后、`turnstile` 前添加 `pub mod patches;`，并添加文档注释 `/// 反检测 JS 补丁注入。`（消除 `missing_docs` lint warning）
- `src/browser/page.rs` 第 116-118 行：`crate::browser::patches::` → `crate::stealth::patches::`
- git 识别为 100% rename

### Task 8: build_stealth_args 拆分（TDD）

1. 添加 4 个新测试（`test_build_common_args_excludes_stealth_specific`、`test_build_stealth_extra_args_contains_stealth_specific`、`test_build_default_args_combines_common_and_stealth`、`test_build_stealth_args_equals_common_plus_stealth_extra`）
2. 验证测试编译失败（`build_common_args` / `build_stealth_extra_args` 未定义）
3. 重构：
   - `build_default_args` 保持不变（调用 `build_stealth_args` 加 `--` 前缀）
   - `build_stealth_args` 改为组合 `build_common_args` + `build_stealth_extra_args`
   - 新增 `build_common_args`：包含所有通用参数（不含 `disable-blink-features=AutomationControlled`），保留 `disable-features=...` 行
   - 新增 `build_stealth_extra_args`：仅包含 `disable-blink-features=AutomationControlled`
4. 验证 10 个测试全部通过

### Task 9: 注释清理

- `src/browser/mod.rs` 第 1 行：`stealth args` → `launch args`
- `Browser::launch` doc comment：`Launch browser with anti-detection patches.` → `Launch browser with given options.` + ARCH 注释说明 Browser 是通用 CDP 层

## 疑虑与观察

1. **预先存在的 warning（与 Task 7/8/9 无关）**：`cargo test --lib` 报告 1 个 warning：`unused import: Browser` 位于 `src/fetcher/strategies/dynamic.rs:128`，由 commit `46e15eb feat: 添加 DynamicStrategy` 引入（PR2 之前），在 `#[ignore]` 标记的集成测试函数 `test_dynamic_strategy_navigates` 内。简报期望"无警告"，但该警告不在 Task 7/8/9 的修改范围内，未处理。

2. **Task 7 文档注释**：简报未明确要求为 `pub mod patches;` 添加文档注释，但因 `lib.rs` 启用 `#![warn(missing_docs)]`，未加注释会触发 warning。为遵守"无警告"约束，添加了 `/// 反检测 JS 补丁注入。` 注释（与原 `browser/mod.rs` 中的注释一致）。

3. **Task 8 git diff 行数**：commit 显示 `1 file changed, 72 insertions(+), 4 deletions(-)`，符合预期（4 个新测试约 60 行 + 重构新增约 12 行）。
