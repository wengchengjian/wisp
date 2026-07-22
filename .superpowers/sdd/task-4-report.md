# Task 4 报告：清理 dead_code warnings

- **Status:** DONE
- **Commit:** `49a729b4d5b7bdf826e5ac035a5a01ec55f3befb`

## 测试摘要
- `cargo build --lib` warnings 数量：**0**（Select-String "warning" 输出为空）
- `cargo test --lib`：**164 passed; 0 failed**
- 集成测试 `stop_condition_test` / `builder_api_test` / `multi_spider_test`：**23 passed; 0 failed**（且零 warnings）

## 清理项（7 warnings → 0）

| # | 文件 | 清理内容 |
|---|------|----------|
| 1 | `src/stealth/challenge.rs` | 删除 `wait_js_challenge` / `wait_managed` 两个死方法（约 45 行） |
| 2 | `src/browser/page.rs` | 删除 `Page.headless` 死字段 + 构造赋值（参数保留，`create()` 内仍用 `headless` 决定 stealth 脚本与 UA） |
| 3 | `src/browser/mod.rs` | 删除 `use std::os::windows::process::CommandExt;`（tokio `Command` 原生提供 `creation_flags`） |
| 4 | `src/fetcher/mod.rs` | 删除 `use serde_json::Value;` + 从 `crate::error::{WispError, Result}` 移除 `WispError` |
| 5 | `src/fetcher/session.rs` | 移除 `let mut resp` 的 `mut` |
| 6 | `src/crawl/engine.rs` | `final_resp` 去掉 `= None` 初始化（从未读取），进而去掉 `mut`（两分支均为首次赋值/definite assignment） |
| 7 | `tests/builder_api_test.rs` | 删除 5 个 unused imports：`HashSet` / `async_trait` / `ClosureSpider` / `wisp::http` / `FetchMode` |

## 额外说明
- `engine.rs` 的 `final_resp`：去掉 `= None` 后编译器进一步提示 `mut` 不需要（两个分支的赋值均被视为初始化而非修改），故一并移除 `mut`。`last_error` 保留 `= None` 与 `mut`，因为 cache-hit 分支不赋值而依赖初始值被后续读取。
- `browser/mod.rs` 的 `CommandExt`：tokio 的 `tokio::process::Command` 在 Windows 上原生提供 `creation_flags` 方法，无需导入 std trait。
- `tests/real_scrape_test.rs`、`tests/cf_bypass_real_test.rs`、`tests/session_test.rs` 的 GBK 编码问题为预先存在，本次未触碰，不在本任务范围。

## Concerns
无。所有 warnings 均已清理，无遗留。
