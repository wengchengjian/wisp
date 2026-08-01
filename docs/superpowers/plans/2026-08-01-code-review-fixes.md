# Code Review 修复计划

> **For agentic workers:** 按本计划逐任务执行；每个任务独立测试与提交。

**Goal:** 修复 code review 发现的阻断项与行为缺陷，清理结构性冗余，使全 feature 组合测试可编译、核心统计与安全承诺生效。

**Architecture:** 12 个独立提交任务：先恢复测试与 feature 编译（1-2），再修行为缺陷（3-9），最后结构清理与文档同步（10-12）。

**Tech Stack:** Rust 2021 workspace、cargo、wreq 6.0.0-rc.29、hickory-resolver 0.24、serde_json/bincode、tokio。

## Global Constraints

- 变量命名 snake_case；提交信息使用中文。
- 每个任务只 `git add` 本任务涉及文件，不提交无关工作树改动。
- 公共 API 仅在 Task 10/11 标注处变更。
- 命令在仓库根 `F:\project\wisp` 执行。
- 真实网络/Chrome 测试保持 `#[ignore]`。

## Task 1: 修复 fetcher/storage 库测试导入

- 修改 `crates/storage/src/sqlite/tests.rs`、`crates/fetcher/src/cookie/browser/tests.rs`、`crates/fetcher/src/cookie/cf/mod.rs`、`crates/fetcher/src/client/mod.rs`
- 补齐 `crate::Store`、`serde_json::json`、`Cookie/CookieJar/Url/Duration`、`Request/Response/Result` 导入
- 验证：`cargo test --workspace --all-features --tests --no-run --no-fail-fast`
- 提交：`fix: 补齐模块拆分后测试导入，恢复库测试编译`

## Task 2: 集成测试加 browser feature gate

- `tests/cr_fix_pool_test.rs`、`tests/stealth.rs`、`tests/browser_status_code_test.rs` 首行加 `#![cfg(feature = "browser")]`
- 验证：`cargo check --workspace --no-default-features --all-targets`
- 提交：`fix: 集成测试按 browser feature 门控，修复无默认特性构建`

## Task 3: offsite 统计真实计数

- `crates/crawl/src/engine/request.rs` 拆出 `is_allowed_domain`，拒绝时 `stats.offsite.fetch_add`
- `crates/crawl/src/engine/tests.rs` 加 `DomainRestrictedSpider` 与计数测试
- 验证：`cargo test -p wisp-crawl --lib`
- 提交：`fix: 站外域名过滤时递增 offsite 统计`

## Task 4: DoH 配置真实接线

- `crates/http/Cargo.toml` 加 `hickory-resolver`
- 新建 `crates/http/src/dns.rs`：`DoHResolver` 实现 `wreq::dns::Resolve`
- `crates/http/src/lib.rs` 导出；`crates/http/src/builder.rs` build 时 `dns_resolver`
- 验证：`cargo test -p wisp-http --lib`
- 提交：`feat: DNS-over-HTTPS 配置接入 wreq 自定义 resolver`

## Task 5: MCP 工具 SSRF 校验补齐

- `crates/mcp/src/tools/fetch.rs`、`adaptive.rs` 入口调 `validate_url`
- `crates/mcp/src/tests.rs` 加内网拒绝测试
- 验证：`cargo test -p wisp-mcp --lib`
- 提交：`fix: MCP fetch_page/adaptive_scrape 补 SSRF URL 校验`

## Task 6: 多 Spider checkpoint 恢复去重

- `crates/crawl/src/runner/setup/checkpoint.rs` 加 `merge_checkpoint_states`，恢复时单次导入
- `crates/crawl/src/runner/tests.rs` 加去重测试
- 验证：`cargo test -p wisp-crawl --lib`、`cargo test --test run_inner_test`
- 提交：`fix: 多 Spider checkpoint 恢复按 URL 去重，避免重复爬取`

## Task 7: sanitize_url 凭据脱敏修复

- `crates/core/src/utils/url.rs` 用 url crate 构造脱敏
- 加密码含 `@`、query 保留、仅 username 测试
- 验证：`cargo test -p wisp-core --lib`
- 提交：`fix: sanitize_url 完整脱敏密码并保留 query 原值`

## Task 8: WARC 输出二进制安全

- `crates/crawl/src/runtime/output.rs` `to_warc_record` 返回 `Vec<u8>`，清洗 CRLF
- `write_response` 写 record + `\r\n\r\n`
- 验证：`cargo test -p wisp-crawl --lib runtime::output::tests`
- 提交：`fix: WARC 记录保留二进制响应体并清洗 CRLF 注入`

## Task 9: FileStore namespace 路径安全

- `crates/storage/src/file/path.rs` namespace 也走 `sanitize_key`
- `crates/storage/src/file/tests.rs` 加穿越测试
- 验证：`cargo test -p wisp-storage --lib`
- 提交：`fix: FileStore namespace 与 key 统一路径安全化`

## Task 10: 代码清理（小项）

- 清重复 instrument/doc、过时 `allow(dead_code)`、defaults 注释、stealth imports
- MockCookieJar 域名边界、EngineConfig checkpoint 字段、删除 `to_chrome_arg`
- 验证：`cargo check --workspace --all-features --all-targets`
- 提交：`refactor: 清理重复标注、过时注释、未接线字段与无引用 API`

## Task 11: 结构清理（模块命名与死代码）

- `scheduler/scheduler` → `scheduler/core`；`pool/pool.rs` → `pool/core.rs`
- 删除 `restore()`；删除 `config_file`/`WispConfig` 与 toml 依赖
- 验证：`cargo check --workspace --all-features --all-targets`
- 提交：`refactor: 去除同名模块嵌套，删除未接线的 wisp.toml 配置模块`

## Task 12: 文档同步与全量验证

- `docs/known-issues.md` 追加完成记录
- 验证：fmt/check/clippy/test 全量命令
- 提交：`docs: 记录 code review 修复完成项与验证结果`
