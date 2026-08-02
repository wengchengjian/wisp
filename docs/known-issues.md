# wisp 已知问题与剩余工作

> 维护方式：本文档记录当前 `master` 上已验证仍存在的问题、待办决策和后续计划入口。
> 每完成一项，从对应小节删除并补一行完成记录，避免堆积过期条目。

**更新日期：** 2026-08-02
**范围：** master（持续更新）

**完成记录（2026-08-02）：**
- 分页 follow 竞态已修复：`NextWork` 创建时即计入 `global_in_flight`/`stats.in_flight` 并登记
  `in_flight_requests`，避免 `unfold + buffer_unordered` 预填充阶段误判队列空为 Done。
  新增 `tests/follow_race_regression_test.rs` 覆盖单起始 URL 分页；真实 quotes 10 页和 books 3 页均通过。
- 被封锁响应计入成功页已修复：`process_response` 改为响应中间件链通过后再递增 `pages_crawled`/callback 计数，
  `BlockedRetryMiddleware` 重试耗尽时不再误计成功页。新增 `tests/blocked_status_regression_test.rs`；
  真实 403 重试测试通过，e2e 503 仍受 httpbin 外部不稳定影响需单独复查 blocked 统计。
- 缓存回放测试断言已对齐：缓存命中不增加 `pages_crawled`，`crawl_cache_real_test` 和
  `crawl_e2e_real_test` 的 cache replay 断言改为第二次 `pages_crawled=0`，真实回放测试通过。
- Stealth webdriver 测试已对齐实现：`navigator.webdriver` 接受 `undefined` 或 `false`，
  不再强制 `undefined`；真实 Chrome 下 `tests/stealth.rs` 6 项全部通过。
- CF Turnstile 真实绕过已修复：headless 下 Turnstile iframe 内容为空，点击无效；Stealth builder
  现在自动使用 headed + offscreen 真实渲染。新增本地 widget 回归测试，真实 NopeCHA demo 返回
  “NopeCHA - CAPTCHA Demo”页面并通过挑战页断言。
- 真实网络/浏览器测试收口已完成一轮：`cf_bypass_real_test` 10/10、`real_scrape_test` 7/7 通过；
  修复 CF interactive Turnstile 被当作 JS Challenge 空转的问题（按 `_cf_chl_opt.cType` 提前识别），
  并移除 `challenge-platform` 作为挑战页/托管挑战标记的误判。
- Dynamic/Stealth 浏览器并发与取消测试已完成：新增 `tests/browser_concurrency_cancel_test.rs`，
  用本地延迟 server 验证 `max_concurrent_pages=2` 上限、Fetcher 取消、Engine abort/shutdown 后池可继续复用；
  8 项真实 Chrome 用例本机手动验证全部通过。
- workspace 元数据与依赖收敛已完成：新增 `[workspace.package]`，剩余直写依赖收进 `[workspace.dependencies]`，
  各 crate 改为 workspace 继承并统一版本管理。
- 语法现代化第一轮已完成：`#[allow]` 全部转 `#[expect]`，可安全改写的 `let_chains` 已压平，
  async closures 用于无捕获/可静态 future 回调；`EventCallback` 支持 `event_listener` 注册 async closure。
  gen blocks 因 stable 1.97 仍 experimental，本轮暂缓。
- loom 模型测试已完成：新增 `crates/crawl/tests/loom_models.rs`，覆盖 scheduler/control/autoscale
  状态不变量；通过 `cargo test -p wisp-crawl --features loom --test loom_models --release` 运行。
- tokio-console 开发观测已接入：根包新增 `console` feature，`novel_profiler` 支持 console 初始化，
  README 已记录运行命令。
**完成记录（2026-08-01）：**

- async `Store` 重构已完成：`Store` trait 与全部自由函数 async 化，SQLite/FileStore 经
  `spawn_blocking` 移出同步 I/O，`parser/adaptive`、`crawl` 缓存/checkpoint、`mcp` 调用点已迁移。
- feature gates 完整下钻已完成：`wisp-browser/wisp-stealth/wisp-mcp` 改为可选依赖并
  按根 feature 条件编译，`wisp-fetcher/crawl` 转发 browser/stealth，`cargo build --no-default-features` 已验证。
- AdaptiveTracker 迁移已完成：持久化职责从 `parser::css_adaptive` 上移到
  `crawl::adaptive::AdaptiveTracker`，parser 恢复纯解析层（移除 storage 依赖），MCP `adaptive_scrape` 已改用 tracker。
- CF UA 一致性修复已完成：`CookieJar` 新增 `ua()`（默认 `None`），`CfCookieJar` 返回会话绑定的
  浏览器 UA，HTTP 快速路径在注入 CF cookie 时同步注入 `User-Agent`。
- 中间件语义修复已完成：动作按阶段拆分为 `RequestMwAction`/`ResponseMwAction`，
  请求阶段不再可能返回 `Refetch`、响应阶段不再可能返回 `Respond`；删除空转的
  DomainFilter/DepthLimit 双实现，域名与深度限制统一由 Spider/Engine 检查。
- 错误传播修复已完成：`fetch_dispatch` 改为返回 `Result<Response, WispError>`，
  `process_error` 接收 `&WispError`（DNS/TLS/代理分类保留到重试决策）；`Spider::handle`
  在 worker 边界隔离 panic；`Page::item` 序列化失败不再写入 `Value::Null`，改为告警并跳过。
- 生命周期语义修复已完成：`shutdown` 明确为优雅关闭（停止调度新请求、等待 in-flight 完成），
  abort 保持立即取消；`CrawlEvent` 的 Error/Retry 发送统一为有界背压 `send().await`，不再静默丢事件。
- 未接线公开能力清理已完成：`EventBus/EngineEvent/Metrics` 接入 Engine 运行路径（启动/调度/响应/
  item/错误/封锁/Auto 升级/并发/checkpoint/完成事件），`EngineBuilder::event_bus/event_listener` 可注册；
  checkpoint 实现 run 前恢复（pending/seen/in-flight/callback_pages 全部持久化，shutdown/abort 保留、自然完成清理）；
  `OutputFormat` 经 `OutputWriterPipeline` 接入 Json/Jsonl/Markdown/Warc 实际输出。
- 多 Spider 路由修复已完成：`Request.spider` 改为稳定 `Option<String>`（spider name），
  `run_stream_many` 支持多 Spider 流式运行并发送 `DoneMany`；middleware/pipeline 生命周期按 Spider 展开，
  autoscaler 聚合全部 Spider stats。
  callback 保留为跨 Spider 路由键（home→detail→chapter 小说爬虫语义）；同名 callback 被多个
  Spider 注册时，未绑定请求显式告警并拒绝，避免静默路由到第一个 Spider。
- 配置与边界语义修复已完成：`FetchClientConfig` 内嵌 `wisp_http::Config`（Deref 兼容旧字段访问），
  `EngineConfig` 公开 transport 配置；`Page.session/session_id` 改为私有字段 + 访问器；
  `Fetcher` 不再静默把 `Auto` 映射为 HTTP，Auto 明确由 crawl Engine 持有。
- 文档与仓库清理已完成：`docs/superpowers` 增加“历史/已废弃”README；`.superpowers/sdd` 从 git
  索引移除并加入 `.gitignore`（磁盘文件保留）；`docs/performance.md` 刷新阶段占比（短采样）
  与完整基准基线（默认 criterion 配置）。
- code review 修复已完成：恢复 fetcher/storage 库测试编译与 no-default 测试目标构建；
  offsite 统计真实计数；DoH 接入 wreq resolver；MCP 工具补 SSRF 校验；
  多 Spider checkpoint 恢复去重；sanitize_url/WARC/FileStore 路径安全修复；
  清理同名模块嵌套与未接线配置模块。
- 半成品与冗余修复已完成：MCP stealth_fetch 复用共享 FetchClient/StealthStrategy；
  crawl_site 真实跟随链接；adaptive_scrape db_path 生效；MCP --db 无 sqlite 时明确降级；
  DomainBlocker 接入 HTTP/浏览器；socks4/socks5 真实分流；浏览器代理认证显式拒绝；
  移除无效 header_order；max_concurrent(0) 构建报错；FetchClient Drop 优雅关闭浏览器池；
  from_client 自动构造 strategy；checkpoint 统计全字段恢复；auto_rules/cf_domain_locks 有界；
  清理 UaRotator/from_stats/user_data_path/过时标注。
- 性能遗留处理：`blocked_reason` 改为 64KB 头部窗口 + ASCII case-insensitive 字节匹配（零分配）；
  header 转换预分配容量；`Page::content_text` 减少逐行分支；wreq 层完成调研并记录

- 长函数/圈复杂度拆分（第一批）：`run_inner_many`/`process_response`/`fetch_dispatch`/
  `fetch_page`/`fetch_page_inner`、Stealth `fetch`、`recv_navigation_status`、MCP `serve`/
  `crawl_site`、autoscaler/robots/adaptive/pipeline/scheduler、core encoding/SSRF/status_text、
  CLI `main` 已拆为职责单一的小函数；`cargo check --all-targets` 与相关库测试通过。

- 长函数/圈复杂度拆分（第二批）：Turnstile 全部主函数、browser installer/CDP/page/launch、parser
  `find_longest_match`/`from_fragment`、MCP `stealth_fetch`/`adaptive_scrape`、Dynamic `fetch`、
  crawl `next_work`/`run_stream_many`/`run_inner_many`/响应中间件继续拆小；全量编译与库测试、
  `run_inner_test` 14 项全部通过。

- 长函数/圈复杂度拆分（第三批）：`fetch_browser_response`/`build_engine_context`/`next_work` 等剩余
  生产编排函数、`detect`/`browse`/`blocked_body_reason`/`status_text_4xx`/`known_alias_encoding` 拆小；
  `novel_profiler`/`bench` 的 `spawn_novel_server` 与页面构建、`scraper_full`/`novel_crawler`/
  `debug_fetch` 示例 main 均拆分；全量编译与库测试、`run_inner_test` 14 项全部通过。

- 长文件模块化：`engine.rs` 拆为 `engine/{context,request,response,fetch,checkpoint,guard}.rs`，
  `middleware/builtin.rs` 拆为 `builtin/{request,challenge,limit,upgrade,retry,defaults}.rs`，
  `runner.rs` 拆出 `runner/{work,builder}.rs`；公共路径与行为保持不变，全量编译与回归测试通过。

- 长文件模块化（第二批）：`browser/page.rs` 拆为 `page/{setup,navigation,evaluate,info,cookies,elements,output}.rs`，
  `parser/lib.rs` 拆出 `node.rs`，`http/lib.rs` 拆为 `config.rs`/`builder.rs`/`client.rs`，
  `fetcher/lib.rs` 拆为 `fetcher.rs`/`builder.rs`；公共 API 路径与行为保持不变，全量编译与库测试通过。

- 长文件模块化（第三批）：`crawl/builder.rs` 拆为 `builder/{mod,spider,closure}.rs`，
  `browser/patches.rs` 拆为 `patches/{mod,scripts}.rs`，`core/types.rs` 拆为 `types/{mod,method,request,response}.rs`；
  公共 API 路径与行为保持不变，全量编译与 core/crawl/browser 库测试通过。

- 长文件模块化（第四批）：`crawl/src/lib.rs` 拆出 `spider.rs`/`crawl_stats.rs`/`crawl_stream.rs`，
  `parser/node.rs` 拆为 `node/{mod,single,list}.rs`，`crawl/auto.rs` 拆为 `auto/{mod,generalize,engine,blocked}.rs`；
  公共 API 路径与行为保持不变，全量编译与 crawl/parser 库测试通过。

- 长文件模块化（第五批）：`runtime/robots.rs` 拆为 `robots/{mod,parser,cache}.rs`；
  `middleware/builtin.rs` 与 `engine.rs` 的大测试块分别移到 `builtin/tests.rs`/`engine/tests.rs`，
  主文件只剩模块声明和 re-export；公共路径不变，全量编译与 crawl 125 项库测试通过。

- 长文件模块化（第六批）：`runner.rs` 拆出 `runner/{setup,lifecycle}.rs`（Engine 主体保留公共入口），
  `engine/fetch.rs` 拆为 `engine/fetch/{mod,dispatch,page}.rs`，`middleware/pipeline.rs` 拆为
  `pipeline/{mod,jsonl,filter,batch,output}.rs`；公共路径不变，全量编译与 crawl 125 项库测试通过。

- 长文件模块化（第七批）：`fetcher/client.rs` 拆为 `client/{mod,config,fetch_client}.rs`，
  `browser/installer.rs` 拆为 `installer/{mod,locate,version,download,extract}.rs`，`stealth/turnstile.rs` 拆为
  `turnstile/{mod,config,solve,check,click}.rs`；公共路径不变，全量编译与 fetcher/browser/stealth 库测试通过。

- 长文件模块化（第八批）：`fetcher/cookie/cf.rs` 拆为 `cookie/cf/{mod,jar,session}.rs`，
  `http/block.rs` 拆为 `block/{mod,blocker,ad_domains}.rs`，`scheduling/scheduler.rs` 拆为
  `scheduler/{mod,dedup,queue,scheduler}.rs`；公共路径不变，全量编译与 fetcher/http/crawl 库测试通过。

- 长文件模块化（第九批）：`storage/src/lib.rs` 拆出 `store.rs`/`models.rs`/`functions.rs`，
  `fetcher/cookie/http.rs` 拆为 `cookie/http/{mod,jar}.rs`，`crawl/engine/response.rs` 拆为
  `engine/response/{mod,middleware,handler,emit}.rs`；公共路径不变，全量编译与
  storage/fetcher/crawl 库测试通过。

- 长文件模块化（第十批）：`mcp/tools.rs` 拆为 `tools/{mod,spider,fetch,extract,crawl,adaptive,stealth}.rs`，
  `core/encoding.rs` 拆为 `encoding/{mod,decode,detect,labels}.rs`，`browser/pool.rs` 拆为
  `pool/{mod,pool,handle}.rs`；公共路径不变，全量编译与 mcp/core/browser 库测试通过。

- 长文件模块化（第十一批）：`middleware/mod.rs` 拆为 `middleware/{mod,actions,context,traits,chain}.rs`，
  `fetcher/strategies/stealth.rs` 拆为 `stealth/{mod,cookie,navigation,extract,tests}.rs`，
  `runtime/autoscale.rs` 拆为 `autoscale/{mod,config,metrics,policy,worker,tests}.rs`，
  `browser/lib.rs` 拆出 `process.rs`/`tests.rs`；公共路径不变，全量编译与 crawl/fetcher/browser 库测试通过。

- 长文件模块化（第十二批，10 个文件）：`crawl/lib.rs` 拆出 `tests.rs`，`builder/mod.rs` 拆出 `builder/tests.rs`，
  `observability/events.rs` 拆为 `events/{mod,event,bus,listener,metrics,tests}.rs`，`page.rs` 拆为
  `page/{mod,query,meta,links,items,tests}.rs`，`storage/file.rs` 拆为 `file/{mod,path,ttl,io,tests}.rs`，
  `http/client.rs` 拆为 `client/{mod,headers,request,response,error}.rs`，`fetcher/cookie/browser.rs` 拆为
  `cookie/browser/{mod,jar,tests}.rs`，`engine/fetch/page.rs` 拆为 `fetch/page/{mod,auto_mode,http,browser}.rs`，
  `mcp/lib.rs` 拆出 `protocol.rs`/`server.rs`/`tests.rs`，`src/bin/wisp.rs` 迁为 `src/bin/wisp/{main,browser,scrape,mcp}.rs`；
  公共路径不变，全量编译与 storage/http/crawl/fetcher/mcp 库测试通过。

- 长文件模块化（第十三批，10 个文件）：`parser/node/single.rs` 拆为 `node/single/{mod,fragment,query,navigation,find}.rs`，
  `storage/sqlite.rs` 拆为 `sqlite/{mod,schema,kv,tests}.rs`，`storage/lib.rs` 拆出 `tests.rs`，
  `fetcher/strategy.rs` 拆为 `strategy/{mod,event,extract,tests}.rs`，`browser/cdp.rs` 拆为
  `cdp/{mod,event,connection,command,wait}.rs`，`stealth/challenge.rs` 拆为 `challenge/{mod,detect,solve,tests}.rs`，
  `crawl/adaptive.rs` 拆为 `adaptive/{mod,tracker,convert,tests}.rs`，`crawl/builder/spider.rs` 拆为
  `builder/spider/{mod,handlers,sitemap}.rs`，`crawl/runner.rs` 拆出 `runner/{engine,guard,config,tests}.rs`，
  `crawl/runner/work.rs` 拆为 `runner/work/{mod,driver,execution,scheduling}.rs`；公共路径不变，
  全量编译与 parser/storage/fetcher/browser/stealth/crawl 库测试通过。

- 长文件模块化（第十四批，10 个文件）：`core/error.rs` 拆为 `error/{mod,browser,network,parse,mcp,storage}.rs`，
  `runtime/robots/mod.rs` 拆出 `robots/tests.rs`，`browser/launch.rs` 拆为 `launch/{mod,resolve,args,tests}.rs`，
  `fetcher/cookie/mod.rs` 拆出 `cookie/{types,contract,mock,tests}.rs`，`parser/difflib.rs` 拆为
  `difflib/{mod,matcher,algorithm,tests}.rs`，`parser/adaptive.rs` 拆为 `adaptive/{mod,snapshot,helpers,score,relocate}.rs`，
  `stealth/human.rs` 拆为 `human/{mod,mouse,keyboard,scroll,browse,math}.rs`，`runner/builder.rs` 拆为
  `builder/{mod,methods,build}.rs`，`runner/setup.rs` 拆为 `setup/{mod,checkpoint,start,middleware,context}.rs`，
  `scheduler/scheduler.rs` 拆为 `scheduler/{mod,snapshot,restore}.rs`；公共路径不变，
  全量编译与 core/parser/storage/fetcher/browser/stealth/crawl 库测试通过。

- 过度拆分修正：`core/error.rs`、`stealth/human.rs` 合并回单文件，`fetcher/cookie` 的 types/contract/mock 合并回
  `cookie/mod.rs`（保留独立 `tests.rs`），`parser/difflib` 的 7 行测试内嵌回 `difflib/mod.rs`；
  公共路径不变，core/parser/fetcher/stealth 库测试通过。

---

## 1. master 合并遗留（2026-08-01 整合时未移植）

远程 master 曾在工作期间推进 26 个提交；按确认规则“我们分支优先 + 移植增量”整合，
以下增量明确未移植，需要单独评估：

- **`croner` 依赖移除**：当前无引用；若未来恢复 cron 调度需重新引入。

## 2. 工程化下一阶段优先级（2026-08-02）

> 依据 2026-08-02 重新盘点；已排除已完成项，并保留之前明确不采纳的决策。

### 已完成，不再列入
- Rust 2024 / edition 2024 / stable 1.97.1：已到位；各 crate 已声明 `rust-version = "1.91.1"`。
- CI 基础：`.github/workflows/ci.yml` 已包含 fmt、clippy `-D warnings`、doctest、nextest、cargo deny、llvm-cov 报告。
- workspace 元数据与依赖收敛：`[workspace.package]` + `[workspace.dependencies]` 已统一，公共 package 字段和依赖均以 workspace 继承。
- 测试增强主体：proptest、insta、nextest 已落地；`cargo nextest run --workspace --all-features` 为默认测试命令。
- cargo-deny 已接 CI。


### P2：按风险与 ROI 推进
1. cargo-fuzz：先覆盖 HTML parser、robots、URL、SSRF、proxy config，1-2 个 target 起步。
2. cargo-mutants：只对核心引擎做突变测试，评估现有测试有效性；不纳入常规 CI。
3. 性能工具：已有 Criterion bench，补充 `profile.bench`、Codspeed/Criterion 对比基线，按需使用 cargo-flamegraph / cargo-llvm-lines。
4. MSRV 验证：用 cargo-msrv 测出真实最低版本，再对齐 10 个 crate 的 `rust-version`，不只依赖声明值。

### 明确不采纳
- cargo-semver-checks / cargo-public-api：wisp 仍处快速 API 变动阶段，暂不做。
- cargo-audit：与 `cargo deny check advisories` 重复，暂不做。
- async fn in traits 无脑迁移：项目大量使用 dyn trait object，保留 async-trait；如要演进，再单独评估 trait-variant。

## 3. 剩余工作

- 长函数/圈复杂度拆分第四批（静态扫描剩余约 28 个候选，多为测试内函数与极小高圈复杂 match）：
  `cf_bypass_real_test` 诊断/参数扫描、`make_ctx_*` 测试构造、`default_middlewares_classifies_*` 等。

- 长文件模块化下一批候选（10 个）：`crawl/src/auto/mod.rs`（约 200 行）、`crawl/src/runner/engine.rs`（约 190 行）、
  `fetcher/src/cookie/http/mod.rs`（约 190 行）、`crawl/src/runtime/control.rs`（约 189 行）、
  `fetcher/src/cookie/cf/mod.rs`（约 189 行）、`fetcher/src/fetcher.rs`（约 187 行）、
  `crawl/src/scheduling/stop.rs`（约 181 行）、`core/src/utils/url.rs`（约 180 行）、
  `core/src/encoding/mod.rs`（约 177 行）、`crawl/src/engine/fetch/dispatch.rs`（约 177 行）；
  bench 与补丁脚本文件不列入生产拆分，下一批起阈值下调至约 177 行。

- wreq 6.0.0-rc.29 无条件叠加 Retry/Redirect/Config/Timeout 层；已调研并记录，
  绕开需要换更底层构造（wreq connector/service 或 hyper），暂缓实施。

## 6. 关联计划

- crate 拆分：`docs/superpowers/plans/2026-07-31-crate-split-architecture.md`
- 多 Spider 重构：`docs/superpowers/plans/2026-07-31-multi-spider-refactor.md`
- master arch-refactor 未合入设计：`docs/superpowers/plans/2026-07-26-arch-refactor-pr3-parser-feature.md` 等
