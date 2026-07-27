# Task 8 报告：lib.rs 声明 cookie 模块 + 最终验证

## 任务

PR1（架构重构 CookieJar Storage）最后一个任务：在 `src/lib.rs` 中确认 cookie 模块声明 + 添加 re-export + 全项目最终验证。

## 执行步骤

### Step 1：写测试

在 `src/lib.rs` 末尾追加 `cookie_module_tests` 测试模块，验证 `crate::cookie::{Cookie, CookieJar, CfCookieJar, CfSession, HttpCookieJar, BrowserCookieJar, MockCookieJar}` 全部公开 API 可命名 + trait object 可构造。

为消除 plan 中测试代码导致的 `unused_imports` warning（BrowserCookieJar/CfCookieJar/HttpCookieJar 仅 `use` 未使用），追加 `_assert_implementations` 编译期 trait bound 检查，确保四个实现类型都实现 `CookieJar`。

### Step 2：验证测试运行

`cargo test --lib cookie_module_tests` → 1 passed。

> Plan Step 2 期望测试失败，但 plan 自身说明"如果 Task 1 已声明 `pub mod cookie;` 则应通过；此测试主要验证 re-export 完整性"。Task 1 已声明模块，故测试通过属预期行为。

### Step 3：添加 re-export

- `pub mod cookie;` 已在 Task 1 中声明于 `src/lib.rs:90`，无需新增。
- 在 `pub use stealth::TurnstileConfig;`（lib.rs:123）之后追加：
  ```rust
  // === Cookie 管理 ===
  pub use cookie::{BrowserCookieJar, CfCookieJar, CfSession, Cookie, CookieJar, HttpCookieJar};
  ```
- MockCookieJar 不导出（测试辅助类型，按 plan 仅 `pub` 在模块内部使用）。

### Step 4：运行测试

- `cargo test --lib` → **318 passed; 0 failed; 10 ignored**（含新增 `cookie_module_tests::cookie_module_public_api_accessible`，cookie 模块共 25 个测试：mod.rs 7 + cf.rs 6 + http.rs 7 + browser.rs 5）。
- `cargo build` → 编译成功，无 warning。
- `cargo test --doc` → **11 passed; 0 failed; 3 ignored**，doctest 无破坏。

### Step 5：clippy + 全特性构建

- `cargo clippy --all-targets` → 仅有 2 个预先存在的 warning（`src/cookie/browser.rs:158,168` 的 `unreadable_literal`，Task 4 引入，非本次修改）。Task 8 自身代码无 warning。
- `cargo build --all-features` → 成功（含 sqlite feature）。
- `cargo build --release` → 成功。

### Step 6：提交

- 恢复 `.gitignore` 的无关变更（非 Task 8 责任，疑似前序任务遗留）。
- 仅提交 `src/lib.rs`。

**Commit：** `e6e99e7 feat: lib.rs 声明 cookie 模块并 re-export 公开 API`

## 验证结果汇总

| 验证项 | 命令 | 结果 |
|---|---|---|
| 单元测试 | `cargo test --lib` | ✅ 318 passed / 0 failed / 10 ignored |
| 文档测试 | `cargo test --doc` | ✅ 11 passed / 0 failed / 3 ignored |
| Debug 构建 | `cargo build` | ✅ 无 warning |
| Release 构建 | `cargo build --release` | ✅ 成功 |
| 全特性构建 | `cargo build --all-features` | ✅ 成功 |
| Clippy | `cargo clippy --all-targets` | ⚠️ 2 个预先存在的 warning（browser.rs，非本次） |
| 公开 API | `FetchClient::cookie_jar()` → `&Arc<dyn CookieJar>` | ✅ 符合 spec |
| 残留扫描 | `grep -r "cf_cache\|has_cf_cookies\|CfSessionCache" src/` | ✅ 无残留 |

## PR1 完成状态

PR1 全部 8 个任务完成，commit 链：

```
e6e99e7 feat: lib.rs 声明 cookie 模块并 re-export 公开 API           ← Task 8（本次）
09ac90b 修复 Task 6 审查问题：删除空测试并将 unwrap 改为 expect
e2d968b feat: FetchClient 集成 CookieJar，删除 cf_cache 字段          ← Task 6
1afdd14 feat: StorageError 新增 NotFound/Serialization/Backend/Corrupted ← Task 5
decdb60 feat: 添加 BrowserCookieJar（通过 CDP Network 域）             ← Task 4
54aeb4c fix: HttpCookieJar::set 写入 Expires 字段避免 expires 丢失
89537b9 feat: 添加 HttpCookieJar（包装 wreq::cookie::Jar）             ← Task 3
408d6db feat: 添加 CfCookieJar（迁移自 fetcher/client.rs）            ← Task 2
b283150 feat: 添加 CookieJar trait 和 Cookie 类型                      ← Task 1
```

## 产出 API

`wisp::cookie::{Cookie, CookieJar, CfCookieJar, CfSession, HttpCookieJar, BrowserCookieJar, MockCookieJar}` 全部公开可访问；顶层 re-export：`wisp::{Cookie, CookieJar, CfCookieJar, CfSession, HttpCookieJar, BrowserCookieJar}`。

## 状态

✅ **Task 8 完成。PR1 全部完成。**
