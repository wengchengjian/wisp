# PR2 Task 6 实施报告：Fetcher 持有 browser_strategy 字段

## 任务状态

**DONE_WITH_CONCERNS**

任务完成，所有测试通过，但有一处对简报的偏差（详见"疑虑"部分）。

## 提交信息

- **Commit hash**: `5e3073f85d6621007c22008874cbbb95ee3f5d8a`
- **提交信息**: `refactor: Fetcher 持有 browser_strategy，按 mode 自动构造`
- **修改文件**:
  - `src/fetcher/mod.rs`（按简报逐字修改）
  - `src/error.rs`（添加 `WispError::Config` 变体，详见疑虑 #1）

## 测试运行结果

### 1. `cargo test --lib fetcher`

```
test result: ok. 40 passed; 0 failed; 2 ignored; 0 measured; 303 filtered out; finished in 5.01s
```

新增的 4 个测试全部通过：
- `test_fetcher_http_mode_has_no_strategy` ... ok
- `test_fetcher_auto_mode_has_no_strategy` ... ok
- `test_fetcher_dynamic_mode_has_strategy` ... ok
- `test_fetcher_stealth_mode_has_strategy` ... ok

### 2. `cargo build --lib`

```
Finished `dev` profile [unoptimized +debuginfo] target(s) in 8.88s
```

无警告。

### 3. `cargo test --lib`（全量）

```
test result: ok. 333 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out; finished in 5.25s
```

无回归。

## 实施摘要

按简报逐字实施：

1. **测试模块**：在 `src/fetcher/mod.rs` 的 `#[cfg(test)] mod tests` 中添加 4 个新测试，验证各 mode 下 `browser_strategy` 字段的正确性。
2. **结构体**：给 `Fetcher` 添加 `browser_strategy: Option<Arc<dyn BrowserFetchStrategy>>` 字段。
3. **`new`**：调用 `Self::build_strategy(mode, &config)` 构造 strategy。
4. **`build_strategy`**：私有方法，按 mode 构造 strategy（Http/Auto → None，Dynamic → DynamicStrategy，Stealth → StealthStrategy + CfCookieJar）。
5. **`from_client`**：设置 `browser_strategy: None`。
6. **`fetch`**：使用 `self.browser_strategy.as_ref()` 获取 strategy，若为 None 返回 `WispError::Config`。
7. **访问器**：添加 `browser_strategy()` 返回 `Option<&Arc<dyn BrowserFetchStrategy>>`。
8. **import**：在 `mod.rs` 顶部添加 `use crate::error::{Result, WispError};` 和 `use crate::fetcher::strategy::BrowserFetchStrategy;`。

## 疑虑与观察

### 1. 简报引用了不存在的 `WispError::Config` 变体（已解决）

简报 Step 3.5 中 `fetch` 方法使用：

```rust
WispError::Config(format!(
    "{:?} mode requires browser_strategy, use Fetcher::new() instead of from_client()",
    self.mode
))
```

但 `WispError` 中**不存在 `Config` 变体**。简报明确要求"逐字复制"该代码，因此为使简报代码可编译，我在 `src/error.rs` 中添加了 `Config(String)` 变体：

```rust
/// 配置 / 使用方式错误（如 `Fetcher::from_client` 创建 Dynamic/Stealth 模式后调用 fetch）。
#[error("Config error: {0}")]
Config(String),
```

这是超出"只改 `src/fetcher/mod.rs`"约束的最小必要修改。如果不添加此变体，简报代码无法编译，任务无法完成。

**替代方案（未被采用）**：使用现有的 `WispError::Engine(String)` 替代，但这语义错误（`Engine` 是 ND-001-ARCH 引擎状态错误，非配置错误）。

### 2. 预先存在的警告（未修复，与本任务无关）

`cargo test --lib` 时有一个预先存在的警告，**不是本任务引入的**：

```
warning: unused import: `Browser`
   --> src/fetcher/strategies/dynamic.rs:128:30
```

该警告位于 `dynamic.rs` 的 `#[ignore]` 集成测试中，`use crate::browser::{Browser, BrowserPool};` 中 `Browser` 未被使用。按 CLAUDE.md "精准修改"原则（"如果注意到无关的死代码，提一下 —— 不要删除它"），未修复。

### 3. `Fetcher::from_client` 的语义行为

按简报，`from_client` 设置 `browser_strategy: None`。如果使用 `from_client` 创建 Dynamic/Stealth 模式的 Fetcher 再调用 `fetch`，会返回 `WispError::Config`。这是预期行为，已在 `from_client` 文档注释中说明。

### 4. engine.rs 未修改

按任务约束，未修改 `src/crawl/engine.rs`。`engine.rs` 中的 `fetch_page_inner` 直接使用 `fetch_client.fetch_browser(...)`，不通过 `Fetcher`，因此不受本任务影响。
