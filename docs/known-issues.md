# wisp 已知问题与剩余工作

> 维护方式：本文档记录当前 `master` 上已验证仍存在的问题、待办决策和后续计划入口。
> 每完成一项，从对应小节删除并补一行完成记录，避免堆积过期条目。

**更新日期：** 2026-08-01
**范围：** master `430c37d`（crate 拆分 + arch-refactor 整合后）

---

## 1. master 合并遗留（2026-08-01 整合时未移植）

远程 master 曾在工作期间推进 26 个提交；按确认规则“我们分支优先 + 移植增量”整合，
以下增量明确未移植，需要单独评估：

- **async `Store` 重构**：master 曾把 `Store` 改为 async trait 并用
  `spawn_blocking` 移出同步 I/O（见 `src/storage/mod.rs` 历史 diff）。当前
  `crates/storage/src/lib.rs` 仍是同步 trait，SQLite/FileStore 的同步 I/O 会阻塞
  async worker。迁移会连带 `parser/adaptive`、`middleware`、`checkpoint` 全部调用点。
- **AdaptiveTracker**：master 计划
  `docs/superpowers/plans/2026-07-26-arch-refactor-pr3-parser-feature.md` 曾把
  `css_adaptive` 迁到 `crawl::adaptive::AdaptiveTracker`；代码未合入（合并按“我们分支
  优先”丢弃），当前仍为 `parser::css_adaptive`。两个方向功能重复，需二选一。
- **feature gates 仅声明未完整下钻**：根 `Cargo.toml` 默认全开
  `http/browser/stealth/mcp`，`wisp-fetcher` 有 `browser/stealth` feature，但
  `wisp-browser`、`wisp-stealth`、`wisp-mcp` 模块尚未按 feature 条件编译；
  `cargo build --no-default-features` 未验证。
- **CF UA 一致性信息丢失**：旧 `has_cf_cookies/get_cf_cookie_header/get_cf_ua`
  已被 `cookie_jar().header()` 简化（`crates/crawl/src/engine.rs`），HTTP 快速路径
  不再注入浏览器实际 UA；真实 CF 站点回退行为需要实测验证。
- **`croner` 依赖移除**：当前无引用；若未来恢复 cron 调度需重新引入。

## 2. 架构审核遗留（crate 拆分后仍存在）

### 2.1 未接线/半成品公开能力

- **EventBus / EngineEvent / Metrics**：
  `crates/crawl/src/observability/events.rs:25`（事件枚举）、`:104`（EventBus）、
  `:199`（Metrics）。仅有单元测试调用，Engine 运行路径未接入；不接线就应从公开面删除。
- **Checkpoint**：`EngineBuilder::checkpoint` 字段存在
  （`crates/crawl/src/runner.rs:35,37,601`），`persist_spider_checkpoint`
  （`crates/crawl/src/engine.rs:601`）只被测试调用（`:990`）。用户已定方向为
  “run 前恢复”，但恢复协议、in-flight 清单与 `until` 计数衔接尚未实现。
- **OutputFormat / WarcWriter / MarkdownWriter**：
  `crates/crawl/src/runtime/output.rs:13,66,93`，仅自测使用；`JsonlWriterPipeline`
  有真实使用（`examples/novel_crawler.rs`）。前者需接线或删除。

### 2.2 多 Spider 路由

- **`Request.spider` 仍是数组下标**：`crates/core/src/types.rs:87`
  `pub spider: Option<usize>`。checkpoint 恢复或 `run_many` 顺序变化时下标脆弱；
  应改为稳定 `spider_id`/name。
- **`run_stream` 仍单 Spider**：`crates/crawl/src/runner.rs` 的 stream 路径
  只支持单个 Spider；middleware/pipeline 的 init/open/close 用 `spiders[0]` 的
  `CrawlContext`；autoscaler 只采样 `all_stats.first()`。多 Spider 语义只对
  `run_many` 完整。

### 2.3 错误传播

- **错误在引擎边界降级为字符串**：`fetch_dispatch`
  （`crates/crawl/src/engine.rs:402`）返回
  `(Option<Response>, Option<String>)`，`WispError` 的 DNS/TLS/代理分类在重试决策
  处丢失；`process_error` 只收到 `&str`。
- **`Spider::handle` 无 `Result`**：handler panic 会击穿整个 run；需要 worker
  边界 panic 隔离。
- **`Page::item` 序列化失败静默 `Value::Null`**：`crates/crawl/src/page.rs`。

### 2.4 中间件语义

- **动作在不适用阶段被静默吞掉**：`process_request` 收到
  `MwAction::Refetch(_)` 时被忽略（`crates/crawl/src/engine.rs:201-203`）；
  `process_response` 收到 `MwAction::Respond` 落到 `_ => break`
  （`crates/crawl/src/engine.rs:309`）。应按阶段建模动作类型或报错，不能静默降级。
- **DomainFilter/DepthLimit 双实现**：engine 在 `process_request` 顶部直接检查
  `spider.allowed_domains/max_depth`，而 `runner.rs:362-363` 传给
  `default_middlewares` 的 `allowed_domains: Default::default()`、
  `max_depth: u32::MAX`，导致真实链里中间件实例空转/不注入。

### 2.5 生命周期与配置

- **`shutdown` 语义与文档不符**：`crates/crawl/src/runtime/control.rs:85` 注释
  “优雅关闭、in-flight 完成后退出”，但主循环在
  `crates/crawl/src/runner.rs:427` 看到 shutdown 立即 `return None`，
  `buffer_unordered` 会 drop 未完成 future，实际是取消。
- **事件背压不一致**：普通事件 `send().await`，错误/重试用 `try_send`，满时静默丢。
- **传输配置分散**：`http::Config`（`crates/http/src/lib.rs:23`）、
  `FetchClientConfig`（`crates/fetcher/src/client.rs:30`）、`EngineContext`
  （`crates/crawl/src/engine.rs:38`）、`EngineConfig`
  （`crates/crawl/src/runner.rs:98`）多层搬运，加一个选项容易漏同步。
- **`Page.session/session_id` 公开**：`crates/browser/src/page.rs:17,19`，
  跨 crate 组合需要，但把内部实现暴露给公共 API。
- **Auto 两层语义**：`Fetcher::fetch` 对 `Auto` 静默映射 HTTP
  （`crates/fetcher/src/lib.rs`），真正的升级只存在于 crawl 层；新增传输后端要
  改多处。`BrowserFetchStrategy` 已为浏览器模式铺路，Http/Auto 仍未收敛。

## 3. 性能遗留

- **`blocked_reason` 全 body 分配小写**：`crates/crawl/src/auto.rs:205`
  `String::from_utf8_lossy(body).to_lowercase()` 对每个 Auto 响应完整执行；
  检测内容只有几个 ASCII 子串，应改为对 body 头部（如前 64KB）做字节窗口
  case-insensitive 匹配，无分配。
- **wreq 层开销**：`retry::Policy::never()` 已关闭每请求协议重试，但
  timeout/redirect/tower layer 仍在火焰图中；是否可用更底层构造绕开未调研。
- **可继续微调**：`build_fetch_response` header 转换约 2.4%、
  `Page::content_text` 约 2.1%（2026-07-31 profiling 数据）。
- **基准基线**（`docs/performance.md`，2026-08-01）：合并后
  `multi_spider_10books` 中位数 32.928 ms（合并前 32.303 ms，无回归）；
  `http_cached_replay` 9.833 ms 较基线 9.5 ms 有约 3-9% 波动，建议复测确认。

## 4. 文档与仓库清理

- **历史设计文档引用旧 API**：`docs/superpowers/specs/` 与历史 plans 中大量
  `SpiderRequest`、`Session`、`request_cache` 等旧命名；README 已同步新 API，
  历史文档建议标注“历史/已废弃”或归档，避免误导。
- **阶段占比数据过期**：`docs/performance.md` 的 `WISP_TIMING=1` 阶段占比仍是
  2026-07-31 数据，可重跑刷新。
- **`.superpowers/sdd` 过程文件回归 master**：合并副作用把大量 brief/review/diff
  过程文件重新加入 master；建议后续清理或加入 `.gitignore`。

## 5. 建议优先级

- **P0（下阶段计划）**
  1. checkpoint run 前恢复 + in-flight 修复（用户已确认方向）
  2. 错误类型化 + handler panic 隔离
  3. `Request.spider` 改稳定 id
- **P1**
  4. 未接线公开面清理（EventBus / output / checkpoint 二选一）
  5. 中间件动作按阶段类型化 + 消除 DomainFilter/DepthLimit 双实现
  6. async `Store` 迁移
- **P2**
  7. `blocked_reason` 零分配
  8. feature gates 完整下钻
  9. shutdown / 背压语义统一

## 6. 关联计划

- crate 拆分：`docs/superpowers/plans/2026-07-31-crate-split-architecture.md`
- 多 Spider 重构：`docs/superpowers/plans/2026-07-31-multi-spider-refactor.md`
- master arch-refactor 未合入设计：`docs/superpowers/plans/2026-07-26-arch-refactor-pr3-parser-feature.md` 等
