# PR2 Task 5: 删除 recv_navigation_status + extract_browser_response 旧实现

**Files:**
- Modify: `src/fetcher/client.rs`（删除 `recv_navigation_status` 和 `extract_browser_response` 私有方法）
- Test: 无新测试（验证编译 + 现有测试全绿）

**Interfaces:**
- Consumes: Task 1 已在 `strategy.rs` 提供 `recv_navigation_status` 和 `extract_browser_response` 公共 helper
- Produces: 无（仅删除代码）

## 当前状态

Task 4 已完成：
- `do_browser_work_inner` 已删除
- `recv_navigation_status` 和 `extract_browser_response` 仍保留在 `src/fetcher/client.rs`，标记了 `#[allow(dead_code)]`
- 这两个方法是 `FetchClient` 的私有方法，与 Task 1 在 `strategy.rs` 提供的 `pub(crate)` helper 重复

## Step 1: 验证当前状态

Run: `cargo build --lib 2>&1 | grep dead_code`
Expected: 无 dead_code 警告（Task 4 已加 `#[allow(dead_code)]`）

Run: `cargo test --lib fetcher::client`
Expected: PASS（当前测试全绿）

## Step 2: 写最小实现

在 `src/fetcher/client.rs` 中：

1. **删除整个 `recv_navigation_status` 方法**（包括 `#[allow(dead_code)]` 注解和方法体）
2. **删除整个 `extract_browser_response` 方法**（包括 `#[allow(dead_code)]` 注解和方法体）
3. **检查并删除因方法删除而变得无用的 import**：
   - `tokio::sync::broadcast`（如果仅被 `recv_navigation_status` 使用）
   - `crate::browser::cdp::CdpEvent`（如果仅被 `recv_navigation_status` 使用）
   - 其他仅被这两个方法使用的 import

**注意**：不要删除仍被其他代码使用的 import。例如：
- `WispError`、`BrowserError` 仍被 `fetch_browser` 等方法使用
- `serde_json` 仍被其他方法使用
- `Duration` 仍被其他方法使用

## Step 3: 运行测试验证通过

Run: `cargo test --lib fetcher::client && cargo build --lib 2>&1 | grep -E "warning.*dead_code"`
Expected: 测试 PASS；无 dead_code 警告。

Run: `cargo build --lib`
Expected: 无警告无错误。

Run: `cargo test --lib`
Expected: 全量测试无回归。

## Step 4: 提交

```bash
git add src/fetcher/client.rs
git commit -m "refactor: 删除 FetchClient 中已迁至 strategy 的私有方法"
```
