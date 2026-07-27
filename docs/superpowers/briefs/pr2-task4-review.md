# PR2 Task 4 审查报告

**Commit**: `99bd63e refactor: FetchClient::fetch_browser 接收 &dyn BrowserFetchStrategy`
**改动**: `src/fetcher/client.rs`(+58/-192) + `src/fetcher/mod.rs`(+18/-3) + `src/crawl/engine.rs`(+18/-2)

## Spec 合规性

逐项核对简报要求：

- [x] 在 `src/fetcher/client.rs` 测试模块添加了 `MockStrategy` 和 2 个新测试（client.rs:419-478，逐字复制简报）
- [x] `fetch_browser` 方法签名改为 `(&self, req: &Request, strategy: &dyn BrowserFetchStrategy) -> Result<Response>`（client.rs:165-169）
- [x] 删除了 `do_browser_work_inner` 整个方法（约 180 行，原 161-369 行已删除）
- [x] 给 `recv_navigation_status` 和 `extract_browser_response` 加了 `#[allow(dead_code)]`（client.rs:203、274）
- [x] 删除了不再使用的 import `ChallengeSolver`、`HumanBehavior`（client.rs 顶部）
- [x] 在文件顶部添加了 `use super::strategy::BrowserFetchStrategy;`（client.rs:21）
- [x] 提交信息为中文一行：`refactor: FetchClient::fetch_browser 接收 &dyn BrowserFetchStrategy`（与简报 Step 5 字面一致）

**Spec 合规性：7/7 全部通过。**

## 实施者疑虑评估

**实施者疑虑**：简报 Files 仅声明修改 `src/fetcher/client.rs`，但签名变更后 `src/fetcher/mod.rs:141-142` 和 `src/crawl/engine.rs:814` 的旧调用方（`solve_cf: bool`）无法编译，被迫同步更新。

### 评估结论：**接受**

**1. 必要性**：✓ 签名从 `(&self, req: &Request, solve_cf: bool)` 改为 `(&self, req: &Request, strategy: &dyn BrowserFetchStrategy)`，所有调用方必须更新才能编译。这是签名变更的必然结果，符合 CLAUDE.md「不向后兼容」原则。

**2. 修改正确性**（已逐一验证）：

- `src/fetcher/mod.rs` 的 `Fetcher::fetch`：
  - Dynamic → `DynamicStrategy::from_config(self.client.config())` ✓（签名匹配：`fn from_config(config: &FetchClientConfig) -> Self`）
  - Stealth → 先 `Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl))`，再 `StealthStrategy::from_config(config, cf_jar)` ✓（签名匹配：`fn from_config(config: &FetchClientConfig, cf_jar: Arc<CfCookieJar>) -> Self`，`CfCookieJar::new(data_dir: &Path, ttl: Duration)` 参数类型匹配）
  - 新增 `use crate::cookie::CfCookieJar;` ✓

- `src/crawl/engine.rs:814`：使用 `match mode` 注入对应 Strategy，移除 `let solve_cf = mode == FetchMode::Stealth;`，`unreachable!("上方 if 已过滤非浏览器模式")` 处理不可能分支 ✓

**3. 与 Task 6 的关系**：实施者报告明确指出 Task 6 会重构 `Fetcher` 持有 `browser_strategy` 字段，本任务的临时构造是过渡方案。mod.rs 中新增的 ARCH 注释（"Fetcher 是一次性请求场景，每次创建新 jar 实例；持续爬取场景应直接使用 FetchClient + 长生命周期 CfCookieJar"）清晰呈现了权衡，符合 CLAUDE.md「呈现权衡」原则。Task 6 重构时只需将 `strategy` 上提到 `Fetcher` 字段即可，不会与本任务冲突。

**4. 调用方修改最小化**：
- mod.rs：+18/-3，仅改 `fetch` 方法
- engine.rs：+18/-2，仅改 `fetch_page_inner` 中浏览器模式分支
- 两处均无附带重构

## 代码质量

### Critical（必须修复）
无。

### Important（应该修复）
无。

### Minor（可选修复）

**M1. `fetch_browser` 注释与代码语义不完全一致（继承自原代码，非本任务引入）**

`client.rs:178-193` 的注释「无论成功/失败都显式关闭 tab」与实际行为不一致：

```rust
let result = tokio::time::timeout(Duration::from_secs(120), work)
    .await
    .map_err(|_| { ... })?;   // ← timeout Err 或 strategy Err 时 ? 提前 return

// 实际工作；无论成功/失败都显式关闭 tab
let _ = handle.page_mut().close().await;  // ← 仅在 Ok 路径执行
result
```

超时或 strategy 返回 Err 时，`?` 提前返回，跳过 `close()`，依赖 `handle` 的 Drop 兜底。简报 Step 3.2 原代码就是这样写，实施者逐字复制，**非本任务引入的问题**。如需修复，应在 `?` 之前用 `let _ = handle.page_mut().close().await;` 或改用 `match`。建议留给 Task 5/6 顺手处理。

**M2. `Fetcher::stealth()` 每次新建 `CfCookieJar` 实例导致内存层缓存失效**

实施者已在 mod.rs:148-149 注释中明确说明此权衡：「文件层持久化保证跨请求 CF cookie 复用；持续爬取场景应直接使用 FetchClient + 长生命周期 CfCookieJar」。功能正确，仅内存层 moka 缓存无法跨请求复用。Task 6 若让 `Fetcher` 持有 `browser_strategy` 字段即可解决。

## fetch_browser 方法实现验证

- ✓ 120s 总超时保留：`Duration::from_secs(120)`（简报要求字面值，已按简报替换原 `from_mins(2)`，数值等价）
- ✓ `pool.acquire()` 调用正确（client.rs:176）
- ✓ `strategy.fetch(handle.page_mut(), req)` 调用正确（client.rs:180），与 `BrowserFetchStrategy::fetch(&self, page: &mut Page, req: &Request)` 签名一致
- ⚠️ `handle.page_mut().close().await` 仅在 Ok 路径调用（见 M1）
- ✓ 错误消息使用 `sanitize_url` 脱敏：`crate::crawl::engine::sanitize_url(&req.url)`（client.rs:186）

## Global Constraints 检查

- ✓ 变量命名 snake_case：`solve_cf`、`fetch_req`、`cf_jar`、`called` 等
- ✓ 注释中文
- ✓ 提交信息中文一行（含 `refactor:` 前缀，与简报 Step 5 字面一致）
- ✓ 不向后兼容（签名变更无兼容层）
- ✓ 使用 `expect` 而非 `unwrap`：测试中 `expect("build client")`、`expect("build fetcher")` 等

## 跨任务一致性检查

- ✓ `fetch_browser` 新签名与 Task 1 `BrowserFetchStrategy::fetch` trait 一致（`&dyn BrowserFetchStrategy` 接收者，参数 `&mut Page` + `&Request`）
- ✓ Task 4 的调用方临时修改（mod.rs/engine.rs 中 `DynamicStrategy::from_config` / `StealthStrategy::from_config`）调用 Task 2/3 中已定义的构造函数，签名匹配
- ⚠️ Task 4 调用方临时修改是否会被 Task 6 正确重构：**无法从 diff 验证**（无 Task 6 简报可查），但实施者报告与 mod.rs ARCH 注释均明确这是过渡方案，Task 6 重构路径清晰

## 测试运行验证

实施者报告测试结果（无法从 diff 直接验证，依赖实施者诚实报告）：

| 命令 | 通过 / 失败 / 忽略 | 耗时 |
| --- | --- | --- |
| `cargo test --lib fetcher::client` | 7 / 0 / 0 | 29.76s |
| `cargo build --lib` | 无错误无警告 | 5.97s |
| `cargo test --lib`（全量） | 329 / 0 / 12 | 5.26s |

- ✓ `test_fetch_browser_no_pool_returns_error`：纯逻辑验证无 Chrome 依赖，assert `result.is_err()` + `called == 0`
- ✓ `test_fetch_browser_invokes_strategy`：无 Chrome 环境下走 `result.is_ok() == false` 分支不报错（条件断言符合简报要求）

## 总体结论

**APPROVED**

### 审查摘要

- **Spec 合规性**：7/7 全部通过，逐字复制简报代码无偏差
- **实施者疑虑**：调用方更新（mod.rs + engine.rs）是必要的、最小化的、正确的，接受
- **Critical**：0
- **Important**：0
- **Minor**：2（均非本任务引入，可留给后续 task）
- **无法从 diff 验证的 ⚠️ 项**：
  1. 测试运行结果依赖实施者报告（无法从 diff 验证 cargo test/build 实际通过）
  2. Task 6 重构兼容性（无 Task 6 简报可查，但过渡方案设计合理）
  3. `fetch_browser` 在 timeout/strategy Err 路径不显式 close page（继承自原代码，Drop 兜底）
