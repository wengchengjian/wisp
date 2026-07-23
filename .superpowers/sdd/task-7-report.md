# Task 7 Report: 修复 robots.txt 端口丢失与失败缓存

## Status
✅ COMPLETE — `cargo build` 通过；`cargo test --lib` 201 passed；`cargo test --test cr_fix_robots_port_test` 2 passed；现有 robots 单元测试 12 passed。

## Commit
`bcc90ba` — `fix(robots): 保留端口 + 失败不缓存`
- 分支：`fix/code-review-2026-07-23`
- 文件：`src/crawl/runtime/robots.rs`（+47/-2）、`tests/cr_fix_robots_port_test.rs`（新建，+89）

## 改动摘要
1. **`src/crawl/runtime/robots.rs` — `RobotsRules::is_empty_rules`**（新增 pub 方法）
   `disallowed.is_empty() && crawl_delay.is_none() && request_rate.is_none()`，用于区分"fetch 失败返回的默认空规则"与"成功获取的有效规则"。
2. **`src/crawl/runtime/robots.rs` — `rules_for` domain key 含端口**
   `parsed.port()` 为 `Some(p)` 时拼 `scheme://host:p`，否则 `scheme://host`。修复 `http://example.com:8080` 错误地从 `http://example.com/robots.txt` 获取。
3. **fetch 失败不缓存**
   `if !rules.is_empty_rules() { self.cache.insert(...) }` — 失败返回的空规则不入缓存，下次 `rules_for` 重试。瞬态网络失败不再导致永久"允许全部"。
4. **新增单元测试**：`test_is_empty_rules`、`test_domain_key_preserves_port`（锁定 `url::Url::port()` 行为假设）。

## 测试摘要
| 测试 | 结果 |
|---|---|
| `cargo build` | ✅ |
| `cargo test --lib crawl::runtime::robots` | ✅ 12/12 |
| `cargo test --lib`（全量） | ✅ 201/201 |
| `cargo test --test cr_fix_robots_port_test` | ✅ 2/2 |
  - `robots_fetched_from_correct_port`：mock TcpListener 验证带端口 URL 命中正确端口（counter==1），且 /page 在 `Disallow: /private` 下被允许
  - `fetch_failure_not_cached_so_retry_happens`：先 fetch 死端口（失败不缓存），再 fetch live 端口，counter==1 证明失败未缓存

TDD 流程：先写测试 → 跑确认 2 个 FAIL（端口丢失致 counter==0；失败缓存致第二次仍 counter==0）→ 实现 → 跑确认 PASS。

## API 兼容性
✅ 向后兼容。`RobotsCache` 现有方法签名不变（`is_allowed` / `crawl_delay` / `rules_for` / `new` / `fetch_robots`）。仅新增 `RobotsRules::is_empty_rules` pub 方法，未删除/修改任何现有 API。

## 顾虑 / 取舍
1. **空 robots.txt 不缓存（已知取舍）**：站点 robots.txt 真的为空（无任何 Disallow/Crawl-delay/Request-rate）时，`is_empty_rules` 返回 true，每次 `rules_for` 都重试 fetch。这是 brief 明确接受的取舍（空 robots.txt 少见，重试成本低）。若需精确区分"空规则"与"失败"，需 `fetch_robots` 返回 `Result`，改动更大，超出本 task 范围。
2. **brief 中 mock 断言 bug 已修正**：brief Step 1 的 mock 返回 `Disallow: /` 却断言 `/page` allowed——`/page` 实际匹配 `Disallow: /`（`path.starts_with("/")` 为 true）会被阻止。改为 `Disallow: /private`（Content-Length 同步改为 32），保留 brief "验证 /page 被允许"的意图。
3. **`fetch_failure_not_cached_so_retry_happens` 用 `port+1` 当死端口**：理论上 `port+1` 可能被其他进程占用导致 fetch 成功，但 mock server 已占用 `port`，且测试断言 counter==1（而非 0），若 `port+1` 偶然有 HTTP 服务返回非空规则，counter 仍为 0 → 测试 FAIL（而非误 PASS），不会产生假阳性。

## 报告路径
`/home/weng/wisp/.superpowers/sdd/task-7-report.md`
