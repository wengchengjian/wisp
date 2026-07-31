# PR2 Task 5 报告：删除 recv_navigation_status + extract_browser_response 旧实现

## 任务状态

DONE

## 提交信息

- Commit hash: `720f3b86bb747b8b28ad65a08dfff68b9bbb701f`
- Commit message: `refactor: 删除 FetchClient 中已迁至 strategy 的私有方法`
- 变更: 1 file changed, 117 deletions(-)

## 执行步骤与验证

### Step 1: 删除两个方法

在 `src/fetcher/client.rs` 中删除：
- `recv_navigation_status` 方法（含文档注释、`#[allow(dead_code)]` 注解和方法体）
- `extract_browser_response` 方法（含文档注释、`#[allow(dead_code)]` 注解和方法体）

共删除 117 行。

### Step 2: 检查 unused import

运行 `cargo build --lib` 验证：无警告无错误。

**未删除任何 import**。原因：两个被删除方法内部使用的类型（`tokio::sync::broadcast::Receiver`、`crate::browser::cdp::CdpEvent`、`crate::browser::page::Page`、`serde_json::Value`）均通过全限定路径引用，不在文件顶部的 `use` 语句中。文件顶部所有 import 均被其他代码（如 `fetch_browser`、`FetchClientConfig`、`build_http_client` 等）继续使用。

### Step 3: 测试验证

#### client 单元测试

命令：`cargo test --lib fetcher::client`

结果：
```
running 7 tests
test fetcher::client::tests::test_fetch_client_config_default ... ok
test fetcher::client::tests::fetch_client_config_still_has_cf_fields ... ok
test fetcher::client::tests::test_fetch_client_with_browser_pool ... ok
test fetcher::client::tests::test_fetch_client_http_only ... ok
test fetcher::client::tests::test_fetch_browser_no_pool_returns_error ... ok
test fetcher::client::tests::fetch_client_has_cookie_jar ... ok
test fetcher::client::tests::test_fetch_browser_invokes_strategy ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 334 filtered out
```

#### 全量 lib 测试

命令：`cargo test --lib`

结果：
```
test result: ok. 329 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out
```

无回归。

## 删除的 import

无。两个方法内部使用的类型均通过全限定路径引用，文件顶部 import 全部保留。

## 疑虑与观察

1. **无关警告**：运行 `cargo test --lib fetcher::client` 时观察到 `src/fetcher/strategies/dynamic.rs:128` 有一个 `unused import: Browser` 警告。这是其他任务（可能属于 PR2 其他子任务）的代码，本任务不处理。`cargo build --lib`（非 test 模式）不会触发此警告，因为该 import 在 `#[cfg(test)]` 块或仅在测试构建路径中使用。

2. **删除范围确认**：`strategy.rs` 中的 `recv_navigation_status` 和 `extract_browser_response` 是 `pub(crate)` 函数（Task 1 提供），独立于 `FetchClient`。删除 `FetchClient` 上的私有方法不影响 `strategy.rs` 中的 helper 使用。

3. **纯删除任务**：本任务未添加任何新代码，仅删除重复实现。
