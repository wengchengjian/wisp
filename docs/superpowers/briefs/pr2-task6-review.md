# PR2 Task 6 审查报告

## Spec 合规性

逐项核对简报要求：

- [x] 在 `src/fetcher/mod.rs` 测试模块添加了 4 个新测试（http/auto/dynamic/stealth 各一个）
- [x] `Fetcher` 结构体添加了 `browser_strategy: Option<Arc<dyn BrowserFetchStrategy>>` 字段（含中文文档注释）
- [x] `Fetcher::new` 调用 `Self::build_strategy(mode, &config)` 构造 strategy
- [x] 添加了 `Fetcher::build_strategy` 私有方法（按 mode 构造 strategy）
- [x] `Fetcher::from_client` 设置 `browser_strategy: None`，并补充了文档注释说明限制
- [x] `Fetcher::fetch` 使用 `self.browser_strategy.as_ref()` 获取 strategy，无 strategy 时通过 `ok_or_else` 返回 `WispError::Config`
- [x] 添加了 `browser_strategy()` 访问器（返回 `Option<&Arc<dyn BrowserFetchStrategy>>`，带 `#[must_use]`）
- [x] 提交信息为中文一行：`refactor: Fetcher 持有 browser_strategy，按 mode 自动构造`
- [x] **未修改** `src/crawl/engine.rs`（`git show 5e3073f --stat -- 'src/crawl/engine.rs'` 输出为空，确认未触碰）

### fetch 方法实现验证

- Http/Auto 模式：`self.client.fetch_http(&req).await` ✓
- Dynamic/Stealth 模式：
  - `self.browser_strategy.as_ref()` + `ok_or_else` 返回错误 ✓
  - `self.client.fetch_browser(&req, strategy.as_ref()).await` ✓

### build_strategy 逻辑验证

- Http/Auto → `Ok(None)` ✓
- Dynamic → `Ok(Some(Arc::new(DynamicStrategy::from_config(config))))` ✓
- Stealth → 创建 `CfCookieJar` + `StealthStrategy`，返回 `Ok(Some(Arc::new(...)))` ✓

## 实施者疑虑评估

**WispError::Config 新增：接受**

理由：
1. **必要**：简报 Step 3.5 明确要求"逐字复制"含 `WispError::Config(...)` 的代码，而 error.rs 中确实不存在该变体。不添加则代码无法编译。
2. **无可用替代**：审查 `src/error.rs` 中 `WispError` 全部变体（Browser / Network / Parse / Mcp / Storage / Engine(String) / Timeout(String) / Io）。不存在 `Other(String)` 或类似的通用变体可复用。
3. **语义正确**：`Engine(String)` 注释明确为"ND-001-ARCH：引擎状态错误"，用于并发 run 同一 Engine 场景。`from_client` 缺 strategy 是配置/使用方式错误，语义不匹配，实施者拒绝复用 `Engine` 是正确的判断。
4. **最小化修改**：
   - 仅新增 1 个 `Config(String)` 变体 + 1 行 doc 注释 + 1 行 enum 顶部说明。
   - 无其他改动，符合"精准修改"原则。
5. **风格一致**：与 `Engine(String)` / `Timeout(String)` 同样的 `String` payload + `#[error("Config error: {0}")]` 模式。

## 代码质量

### Critical（必须修复）
无。

### Important（应该修复）
无。

### Minor（可选修复）
1. **`fetch` 方法的 `# Errors` 文档段未更新**：原 `fetch` 的 `# Errors` 注释只提到 `WispError::Network`，新增了 `WispError::Config` 返回路径但未补充文档。非阻塞，可后续补充。
2. **`build_strategy` 中 Stealth 分支的 `CfCookieJar::new` 调用换行风格**：简报原文是单行 `Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl))`，实施者拆为多行。语义等价，仅风格差异，rustfmt 应能接受。

## 跨任务一致性检查

通过 grep 验证关键签名一致性：

- `DynamicStrategy::from_config(config: &FetchClientConfig) -> Self`（dynamic.rs:28）→ 调用 `DynamicStrategy::from_config(config)` 传入 `&FetchClientConfig` ✓ Task 2 一致
- `StealthStrategy::from_config(config: &FetchClientConfig, cf_jar: Arc<CfCookieJar>) -> Self`（stealth.rs:41）→ 调用 `StealthStrategy::from_config(config, cf_jar)` 参数类型匹配 ✓ Task 3 一致
- `CfCookieJar::new(data_dir: &Path, ttl: Duration) -> Self`（cf.rs:40）→ 调用 `CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl)` 签名匹配 ✓ PR1 一致
- `FetchClient::fetch_browser(&self, req: &Request, strategy: &dyn BrowserFetchStrategy)`（client.rs:165）→ 调用 `self.client.fetch_browser(&req, strategy.as_ref())` 中 `strategy.as_ref()` 返回 `&dyn BrowserFetchStrategy`（从 `&Arc<dyn BrowserFetchStrategy>` 解引用）✓ Task 4 一致

## Global Constraints 检查

- 变量命名 snake_case ✓（`browser_strategy`、`cf_jar`、`mode`、`config`）
- 注释中文 ✓（结构体字段注释、`build_strategy` 文档、`from_client` 文档、`browser_strategy` 访问器文档）
- 提交信息中文一行 ✓
- 不向后兼容 ✓（`Fetcher::fetch` 中 Dynamic/Stealth 分支行为变更：从总能构造 strategy 变为依赖字段，外部用 `from_client` 创建的 Dynamic/Stealth Fetcher 现在会失败。这是简报明确要求的行为变更）
- 使用 `expect` 而非 `unwrap` ✓（测试中 4 处 `expect("build fetcher")`）

## Test hygiene

- 4 个测试相互独立，每个都重新构造 `Fetcher`，无共享状态 ✓
- 每个测试都有断言（`is_none()` / `is_some()`）✓
- Dynamic/Stealth 测试带中文失败消息（"Dynamic 模式应有 strategy" / "Stealth 模式应有 strategy"），与简报一致 ✓
- 测试名称与简报逐字一致 ✓
- 报告中声称的测试结果可复现：4 新测试通过，全量 333 passed / 0 failed / 12 ignored 无回归 ✓

## YAGNI 检查

- 未添加简报之外的功能、字段、配置 ✓
- 未引入额外的抽象或可配置性 ✓
- 删除了原 `fetch` 方法 Stealth 分支中的旧注释（"ARCH: StealthStrategy 持有 CfCookieJar..."），这是因代码重写而产生的孤儿注释，删除合理 ✓
- 唯一超出简报的修改是 `error.rs` 中新增 `Config` 变体，已在"实施者疑虑评估"中确认为最小必要 ✓

## 总体结论

**APPROVED**

实施严格按简报逐字执行，仅有 1 处必要偏差（error.rs 新增 `Config` 变体）已正确评估并最小化。所有 9 项 Spec 要求逐条满足，跨任务签名一致，提交信息与命名规范符合 Global Constraints，无 Critical / Important 问题。

