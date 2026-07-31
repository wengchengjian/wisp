# PR2 Task 7+8+9 审查报告

## 审查输入

- 任务简报：`/home/weng/wisp/docs/superpowers/briefs/pr2-task789-brief.md`
- 实施者报告：`/home/weng/wisp/docs/superpowers/briefs/pr2-task789-report.md`
- 审查包：`/tmp/pr2-task789-review.txt`

## 审查方法

- diff 静态审查
- 文件当前状态核对（`src/stealth/mod.rs`、`src/browser/mod.rs`、`src/browser/launch.rs`、`src/browser/page.rs`）
- 运行时验证：`cargo build --lib`、`cargo test --lib browser::launch`、`cargo test --lib stealth::patches`、`cargo test --lib browser::page`
- git history 与 commit 信息核对

---

## Task 7 Spec 合规性

| 项 | 结果 | 证据 |
| --- | --- | --- |
| 创建 `src/stealth/patches.rs`，内容与原 `src/browser/patches.rs` 完全相同 | ✅ | `git show 80d4988` 显示 `src/{browser => stealth}/patches.rs` 100% rename（0 insertions, 0 deletions） |
| 删除 `src/browser/patches.rs` | ✅ | Glob 确认 `src/browser/patches.rs` 不存在；`src/stealth/patches.rs` 存在 |
| `src/browser/mod.rs` 删除 `pub mod patches;` 声明 | ✅ | 当前 `src/browser/mod.rs` 已无 `patches` 模块声明，注释 `/// 反检测 JS 补丁注入。` 也一并删除（符合简报步骤 3） |
| `src/stealth/mod.rs` 添加 `pub mod patches;`（按字母序） | ✅ | 当前顺序：`challenge` → `human` → `patches` → `turnstile`，按字母序正确插入 |
| `src/browser/page.rs` 引用从 `crate::browser::patches::` 改为 `crate::stealth::patches::` | ✅ | `src/browser/page.rs:116,118` 已是 `crate::stealth::patches::HEADLESS_STEALTH_SCRIPT` / `HEADED_STEALTH_SCRIPT`；全仓库 `Grep` 已无 `browser::patches` 残留引用 |
| 提交信息为中文一行 | ✅ | `refactor: browser/patches.rs 迁移到 stealth/patches.rs` |

**Task 7 结论：完全合规。**

---

## Task 8 Spec 合规性

| 项 | 结果 | 证据 |
| --- | --- | --- |
| 添加 4 个新测试 | ✅ | diff 第 114-158 行可见全部 4 个测试，函数名与简报完全一致 |
| 新增 `build_common_args(options: &LaunchOptions) -> Vec<String>` | ✅ | `src/browser/launch.rs:90` 定义，签名匹配 |
| 新增 `build_stealth_extra_args(options: &LaunchOptions) -> Vec<String>` | ✅ | `src/browser/launch.rs:148` 定义，签名匹配 |
| `build_stealth_args` 重构为组合 `build_common_args` + `build_stealth_extra_args` | ✅ | `src/browser/launch.rs:80-84` 函数体为 `let mut args = build_common_args(options); args.extend(build_stealth_extra_args(options)); args` |
| `build_default_args` 保持原有行为（= `build_stealth_args` 加 `--` 前缀） | ✅ | `src/browser/launch.rs:70-75` 未变，仍调用 `build_stealth_args` 后加 `--` 前缀 |
| **关键**：`disable-blink-features=AutomationControlled` 从通用参数移到 stealth 专用参数 | ✅ | `build_common_args` 的 vec 中无此 flag；`build_stealth_extra_args` 的 vec 中有此 flag |
| **关键**：`disable-features=...` 保留在通用参数中 | ✅ | `build_common_args` 仍包含 `disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,...` |
| 提交信息为中文一行 | ✅ | `refactor: build_stealth_args 拆为 build_common_args + build_stealth_extra_args` |

**Test hygiene 核查：**
- ✅ 4 个测试均有 `assert!` / `assert_eq!` 断言
- ✅ 4 个测试相互独立，每个用 `LaunchOptions::default()` 构造自己的 opts
- ✅ 第 4 个测试用 `sort()` 处理顺序差异，断言集合等价（合理）
- ✅ 测试覆盖：通用参数包含/排除、stealth 专用参数包含/排除、`build_default_args` 整合、`build_stealth_args` = common + extra 等价性

**测试运行验证：**
- `cargo test --lib browser::launch`：10 passed; 0 failed（4 新测试 + 6 现有测试）

**Task 8 结论：完全合规。**

⚠️ **无法从 diff 验证**：简报步骤 1-2 要求 TDD 顺序（先写测试 → 验证编译失败 → 再写实现）。diff 只能展示最终状态，无法回溯实施者实际是否遵循 TDD 流程。实施者报告声称已遵循。此项需信任，无法机械验证。

---

## Task 9 Spec 合规性

| 项 | 结果 | 证据 |
| --- | --- | --- |
| 模块注释 `stealth args` → `launch args` | ✅ | `src/browser/mod.rs:1` 已是 `//! Browser process management. Launches Chrome directly with launch args.` |
| `Browser::launch` doc comment 改为 `Launch browser with given options.` 并添加 ARCH 注释 | ✅ | `src/browser/mod.rs:47-50`：`/// Launch browser with given options.` + `/// ARCH: Browser 是通用 CDP 层，不包含反检测逻辑。` + `/// 反检测 JS 补丁由 `stealth::patches` 提供（PR2 已迁移）。` |
| 提交信息为中文一行 | ✅ | `docs: 清理 Browser::launch 注释，移除 anti-detection 描述` |

**Task 9 结论：完全合规。**

---

## 代码质量

### Critical（必须修复）

**无。**

### Important（应该修复）

**无。**

### Minor（可选修复）

1. **`build_default_args` 的 doc comment 描述失真**（简报自身瑕疵，实施者照搬）：
   - 当前注释 `/// Build default Chrome launch arguments from options, with patches applied.` 中 "with patches applied" 具有误导性（此函数仅构建 args，不应用 patches）。
   - 简报的代码示例保留了这行，实施者完全照搬，**不是实施者引入的问题**。
   - 建议后续 PR 清理为 `Build default Chrome launch arguments from options.`，但本次 Task 7/8/9 范围内不强制。

2. **预先存在的 warning**（与本次 3 个 task 无关）：
   - `cargo test --lib` 报告 1 个 warning：`unused import: Browser` 位于 `src/fetcher/strategies/dynamic.rs:128`，由 commit `46e15eb feat: 添加 DynamicStrategy` 引入（PR2 之前），在 `#[ignore]` 标记的集成测试函数 `test_dynamic_strategy_navigates` 内。
   - 简报期望"无警告"，但该警告不在 Task 7/8/9 的修改范围内。
   - 实施者已在报告中明确标注，未擅自处理（符合"精准修改"原则）。
   - 建议单独建 task 处理，不阻塞本 PR。

### YAGNI 核查

- ✅ 未添加简报之外的功能、配置、抽象
- ✅ 未为一次性代码创建抽象
- ✅ 未添加未要求的"灵活性"

### 精准修改核查

- ✅ Task 7 commit 仅触及 4 个文件（patches.rs rename + mod.rs × 2 + page.rs），符合简报
- ✅ Task 8 commit 仅触及 `src/browser/launch.rs`，符合简报
- ✅ Task 9 commit 仅触及 `src/browser/mod.rs`，符合简报
- ✅ 每行修改都能直接追溯到简报要求

### 向后兼容性核查

- ✅ `build_stealth_args` 保留，签名与原一致
- ✅ `Browser::launch` 仍调用 `launch::build_stealth_args(&options)`（`src/browser/mod.rs:78`），无破坏性变更
- ✅ `build_default_args` 行为不变（仍 = `build_stealth_args` + `--` 前缀）
- ✅ `test_stealth_args_has_safe_defaults` 仍通过，证明 `build_stealth_args` 输出包含原所有 flag

### Global Constraints 核查

- ✅ 变量命名 snake_case（`build_common_args`、`build_stealth_extra_args`、`opts`、`args`、`stripped` 等）
- ✅ 注释中文（ARCH 注释、测试注释、模块注释）
- ✅ 提交信息中文一行（3 个 commit 均符合）
- ✅ 不向后兼容：本次为机械重构，无 API 删除；保留 `build_stealth_args` 是因 `Browser::launch` 仍在用，非"为兼容旧代码"的妥协
- ✅ 使用 `expect` 而非 `unwrap`：本次仅测试代码使用 `assert!`/`assert_eq!`，无 `unwrap`/`expect` 引入；生产代码中 `strip_prefix("--").unwrap_or(arg)` 是 `Option::unwrap_or`，非 `Result::unwrap`，无 panic 风险

---

## 实施者疑虑评估

**实施者疑虑**：为 `stealth/mod.rs` 中新增的 `pub mod patches;` 添加了文档注释 `/// 反检测 JS 补丁注入。`，原因是 `lib.rs` 启用了 `#![warn(missing_docs)]`，未加注释会触发 warning。

**评估：接受。**

**理由：**
1. 已核实 `src/lib.rs:36` 确实存在 `#![warn(missing_docs)]`
2. `pub mod patches;` 是公共模块声明，必须配 doc comment 否则触发 missing_docs warning
3. 简报明确要求"无警告"（最终验证步骤）
4. 注释内容 `/// 反检测 JS 补丁注入。` 与原 `browser/mod.rs` 中删除的注释完全一致，保持语义连贯
5. 这是为遵守 Global Constraints 的必要修复，不是简报之外的"功能添加"

---

## 总体结论

**APPROVED**

### 通过项汇总
- Task 7：6/6 spec 项合规
- Task 8：8/8 spec 项合规（含 2 项关键变更）
- Task 9：3/3 spec 项合规
- 代码质量：0 Critical、0 Important、2 Minor（均为预先存在或简报自身瑕疵，非实施者引入）
- Global Constraints：全部满足
- 测试运行：`cargo build --lib` 无警告；`cargo test --lib browser::launch` 10 passed；`cargo test --lib stealth::patches` 3 passed；`cargo test --lib browser::page` 0 passed（无该路径测试，符合简报描述）

### 无法从 diff 验证的 ⚠️ 项
1. **TDD 流程顺序**（Task 8）：简报要求"先写测试 → 验证编译失败 → 再写实现"。diff 只能展示最终状态，无法回溯实施者是否真按 TDD 顺序执行。实施者报告声称已遵循。此项需信任，无法机械验证。**不影响 APPROVED 结论**（最终代码状态正确，测试通过）。
