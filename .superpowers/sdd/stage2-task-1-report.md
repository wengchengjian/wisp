# Stage 2 Task 1 报告：新增 sxd-document/sxd-xpath 依赖与 ParseError 变体

## 任务概述

为 Stage 2（P1 parser 增强）做基础设施准备：在 Cargo.toml 引入 sxd-document / sxd-xpath 两个 crate，并在 WispError enum 中新增 `ParseError(String)` 变体，供后续 XPath 1.0 完整查询懒解析使用。

## 实现内容

### 1. Cargo.toml 修改

在 `chrono = ...` 行之后、`[dev-dependencies]` 之前追加 sxd 依赖（含中文注释）：

```toml
# 时间戳（CrawlState 用）
chrono = { version = "0.4", features = ["serde"] }
# XPath 1.0 完整查询（阶段 2：sxd-xpath 懒解析）
sxd-document = "0.3"
sxd-xpath = "0.4"
```

注释风格与上方既有中文注释一致。

### 2. src/error.rs 修改

在 `WispError` enum 中 `McpError(String)` 变体之后、闭合 `}` 之前追加：

```rust
    #[error("MCP error: {0}")]
    McpError(String),

    #[error("Parse error: {0}")]
    ParseError(String),
}
```

保持与其它变体一致的格式（空行分隔 + `#[error(...)]` 属性 + 元组结构变体）。

## cargo check 输出

```
    Updating `tuna` index
     Locking 5 packages to latest compatible versions
      Adding peresil v0.3.0
      Adding quick-error v1.2.3
      Adding sxd-document v0.3.2
      Adding sxd-xpath v0.4.2
      Adding typed-arena v1.7.0
    Checking sxd-document v0.3.2
    Checking sxd-xpath v0.4.2
    Checking wisp v0.1.0 (F:\project\wisp)
warning: `wisp` (lib) generated 4 warnings (run `cargo fix --lib -p wisp` to apply 1 suggestion)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 15.51s
```

**结果：** exit code 0，编译通过。

**新增传递依赖（5 个）：** peresil v0.3.0、quick-error v1.2.3、sxd-document v0.3.2、sxd-xpath v0.4.2、typed-arena v1.7.0。

**既有 warning（与本次改动无关）：**
- `unused import: std::os::windows::process::CommandExt` (src/browser/mod.rs:55)
- `unused variable: opts` (src/scraper/mod.rs:185)
- `field headless is never read` (src/page/mod.rs:17)
- `methods wait_js_challenge and wait_managed are never used` (src/challenge/mod.rs)

## 文件变更

- `f:\project\wisp\Cargo.toml` — 追加 3 行（1 行注释 + 2 行依赖），净增 3 行
- `f:\project\wisp\src\error.rs` — 追加 3 行（空行 + 属性 + 变体），净增 3 行

合计：2 files changed, 6 insertions(+)

## 自我审查

| 检查项 | 结果 |
|---|---|
| 仅修改 Cargo.toml 和 src/error.rs | ✓ |
| sxd 依赖放置位置正确（chrono 之后、`[dev-dependencies]` 之前） | ✓ |
| 注释风格与上下文一致（中文 # 注释） | ✓ |
| ParseError 变体放置位置正确（McpError 之后、闭合 `}` 之前） | ✓ |
| 变体格式与其它变体一致（空行 + 属性 + 元组变体） | ✓ |
| cargo check 通过 | ✓ exit 0 |
| 未添加测试 | ✓ |
| 未修改其它文件 | ✓ |
| 提交消息为中文 | ✓ |
| 未使用 heredoc（PowerShell 兼容） | ✓ 使用单个 `-m` |

## Commit 信息

- **SHA:** `0f86b547c8cf3d4d013820120bcb68498092241d`
- **Subject:** `feat: 新增 sxd-document/sxd-xpath 依赖与 ParseError 变体`
- **分支:** master
- **基于:** 27f134f（stage 1 cleanup complete）

## 结论

任务 DONE。Stage 2 的依赖与错误变体基础设施已就位，后续 Task 可直接使用 `sxd_document::Document` / `sxd_xpath::evaluate_xpath` 以及 `WispError::ParseError(String)` 返回解析错误。
