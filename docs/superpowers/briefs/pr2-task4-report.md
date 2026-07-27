# PR2 Task 4 报告：FetchClient::fetch_browser 新签名 + 删除 do_browser_work_inner

## 任务状态

**DONE_WITH_CONCERNS**

简报只声明修改 `src/fetcher/client.rs`，但 `fetch_browser` 签名变更后，`src/fetcher/mod.rs` 与 `src/crawl/engine.rs` 的调用方无法编译，必须同步更新到新签名才能让 `cargo test --lib fetcher::client` 通过。这两个文件的改动是被迫的、最小的，符合简报 Step 4 "测试通过" 的成功标准。

## Commit

- Hash: `99bd63edc0f3a8075e37e02c5755be95f63b6993`
- Message: `refactor: FetchClient::fetch_browser 接收 &dyn BrowserFetchStrategy`
- 改动文件：
  - `src/fetcher/client.rs`（+58 / -192）
  - `src/fetcher/mod.rs`（+18 / -3）
  - `src/crawl/engine.rs`（+18 / -2）

## 实施摘要

### client.rs（简报要求）

1. **Step 1**：在 `#[cfg(test)] mod tests` 末尾追加 `MockStrategy` + `test_fetch_browser_invokes_strategy` + `test_fetch_browser_no_pool_returns_error`，逐字复制简报代码。
2. **Step 3.1**：在 `use super::response::{Request, Response};` 后添加 `use super::strategy::BrowserFetchStrategy;`。
3. **Step 3.2**：替换 `fetch_browser` 方法为新签名 `(&self, req: &Request, strategy: &dyn BrowserFetchStrategy)`，调用 `strategy.fetch(handle.page_mut(), req)`，超时从 `Duration::from_mins(2)` 改为 `Duration::from_secs(120)`（数值等价，但简报要求字面值 `from_secs(120)`）。
4. **Step 3.3**：删除 `do_browser_work_inner` 整个方法（原 161-369 行）。
5. **Step 3.4**：给 `recv_navigation_status` 和 `extract_browser_response` 加 `#[allow(dead_code)]` 注解，方法体保持不变（Task 5 才删除）。
6. **Step 3.5**：删除 `use crate::stealth::challenge::ChallengeSolver;` 和 `use crate::stealth::human::HumanBehavior;`（仅被 `do_browser_work_inner` 使用）。

### mod.rs（被迫追加）

`Fetcher::fetch` 原来用 `solve_cf: bool` 调用 `fetch_browser`，签名变更后无法编译。改为按 `FetchMode` 构造对应 Strategy：
- `Dynamic` → `DynamicStrategy::from_config(self.client.config())`
- `Stealth` → 每次创建新 `CfCookieJar`（文件层持久化保证跨请求 CF cookie 复用；Fetcher 是一次性场景，持续爬取应直接用 FetchClient）
- 新增 `use crate::cookie::CfCookieJar;`

### engine.rs（被迫追加）

`fetch_page_inner` 中相同问题。改为 `match mode` 注入对应 Strategy，移除 `let solve_cf = mode == FetchMode::Stealth;`。

## 测试运行结果

| 命令 | 通过 / 失败 / 忽略 | 耗时 |
| --- | --- | --- |
| `cargo test --lib fetcher::client` | 7 / 0 / 0 | 29.76s |
| `cargo build --lib` | 无错误无警告 | 5.97s |
| `cargo test --lib`（全量） | 329 / 0 / 12 | 5.26s |

- `test_fetch_browser_no_pool_returns_error` ✅ 通过（无 Chrome 依赖，纯逻辑验证）
- `test_fetch_browser_invokes_strategy` ✅ 通过（无 Chrome 环境下走 `result.is_ok() == false` 分支，不报错）

## 疑虑与观察

1. **超出简报范围**：简报 Files 仅声明修改 `client.rs`，但实际必须同步修改 `mod.rs` 和 `engine.rs` 才能让 lib 编译通过。已采用最小改动方案，commit message 仍按简报要求。Task 5 若进一步重构（删除 `recv_navigation_status` / `extract_browser_response`），需评估这两个调用方是否还需要进一步调整。

2. **`Fetcher::stealth()` 一次性场景的 CfCookieJar 复用**：当前实现每次 `fetch()` 都新建 `CfCookieJar` 实例。由于 `CfCookieJar` 内部用 moka + 文件持久化，跨请求的 CF cookie 复用通过文件层实现，功能正确但内存层缓存失效。若需内存复用，应在 `Fetcher` 结构体中持有 `Arc<CfCookieJar>` 字段（属 Task 5 范围）。

3. **`dynamic.rs:128` 的 `unused import: Browser` 警告**：来自 Task 2 的 `#[ignore]` 测试 `test_dynamic_strategy_navigates` 中的 `use crate::browser::{Browser, BrowserPool};`。`cargo build --lib` 无此警告（仅 test 配置下出现），符合简报 Step 4 "应无警告" 的预期。未修复（属 Task 2 范围，非本任务造成）。

4. **简报字面值差异**：原 `fetch_browser` 用 `Duration::from_mins(2)`，简报新代码用 `Duration::from_secs(120)`。两者数值等价（120s），已按简报字面值替换。
