# Subagent-Driven Development Progress Ledger

**Plan:** docs/superpowers/plans/2026-07-23-code-review-fixes.md
**Branch:** fix/code-review-2026-07-23
**BASE:** 15021dc

## Tasks
Task 1: complete (commits 15021dc..b06acbf, review clean)
  - 槽位模型修复 retain 移位 + position 错误索引；4/4 pool + 194/194 lib 测试通过
  - Minor（待 Task2/最终review 处理）：acquire 新建路径在锁内 launch（跨 await 持 tokio::Mutex），串行化冷启动；brief 指定，正确性优先
Task 2: complete (commits b06acbf..52b6d9a, review clean)
  - Notify 替换 50ms 轮询；release 先 drop 锁再 notify_one；丢失唤醒循环处理；196/196 通过
  - Minor: 30s 超时硬编码不可配置
Task 3: complete (commits 52b6d9a..083f493, review clean)
  - save_checkpoint 手动构造填 seen_urls；run_inner 用 sched.restore(pending, seen)
  - 预存 Critical 回归修复：SpiderRequest.meta serde(default)→skip（bincode 不支持 deserialize_any，原导致 checkpoint 反序列化全失效）；来源 commit 83cb940（在 base 前最终 review 复查）
  - 197/197 通过
Task 4: complete (commits 083f493..80c76de, review clean)
  - 交换 scale up/down 条件：饱和度>0.9 扩容、<0.7 缩容、错误率高缩容；utilization→saturation 重命名
  - 197/197 通过；Minor: 字段名 cpu_threshold_up/down 与新语义反向（doc 已说明，breaking 改名延后）
Task 5: complete (commits 80c76de..9afa064, review clean)
  - Store::delete_cached_response 真 DELETE；SqliteBackend::delete 改调它；3/3 backend + 6/6 storage 通过
Task 6: complete (commits 9afa064..51c0628, review clean)
  - Node::select 用 let-else 返回空 NodeList；from_fragment 标签非法回退 root_element；22/22 parser + 199/199 lib 通过
Task 7: complete (commits 51c0628..bcc90ba, review clean)
  - rules_for domain key 含 port（http://h:8080 != http://h）；新增 is_empty_rules，fetch 失败返回的空规则不缓存
  - 201/201 lib + 12/12 robots + 2/2 port 测试；实现者修正 brief 测试 bug（Disallow: / 匹配 /page → 改 /private）
Task 8: complete (commits bcc90ba..cad9c82, review clean)
  - RequestCache get/put/invalidate 加 method 参数 + cache_key "{method} {url}"
  - 三调用点同步：engine.rs 查询/写入 + middleware/builtin.rs CacheMiddleware get/put（brief 漏列，编译发现）
  - method_str 上移到 RequestCache 查询前；202/202 + 5/5 + 11/11 通过
Task 9: complete (commits cad9c82..7409460, review clean)
  - resolve_href join 后检查 scheme http/https，过滤 javascript:/mailto:/data:；203/203 通过
Task 10: complete (commits 7409460..17d0716, review clean)
  - build_stealth_args 代理认证配置时 tracing::warn 告知 Chrome --proxy-server 不支持内联认证；proxy-server 仍设置；6/6 + 204/204 通过
Task 11: complete (commits 17d0716..801fa65, review clean)
  - css/xpath_auto/auto_upgrade_check 三处 lock().unwrap()→unwrap_or_else(into_inner)（mod.rs:158/167、engine.rs:440）
  - 新增 spider_response_css_with_tracker_does_not_panic 不回归测试；205/205 lib 通过
  - Minor: xpath_auto 与 auto_upgrade_check 无直接测试（与 css 对称，brief 范围内）；测试为不回归非中毒路径验证（brief 既定）

## All Tasks Complete
- 全部 11 个实现任务完成；下一步：最终全分支 code review（BASE=15021dc，HEAD=801fa65）
- 累积 Minor 待最终 review triage：
  - Task 1: acquire 锁内 launch（跨 await 持 tokio::Mutex，串行化冷启动）
  - Task 2: 30s 超时硬编码不可配置
  - Task 4: 字段名 cpu_threshold_up/down 与新语义反向（doc 已说明）
  - Task 11: xpath_auto/auto_upgrade_check 无直接测试；测试未直接验证中毒路径

## 最终全分支 review（Task 12）
- BASE=15021dc, HEAD=801fa65（review 时）→ 801fa65..c8c0328（fix #1 后）
- 评审结果：With fixes（1 Important + 8 Minor）
- **Important #1（已修）**：Fingerprint 模式 checkpoint seen 恢复失效
  - seen_urls() 返回 u64 哈希字符串；restore() 对其再 fingerprint() 产生不同 u64
  - 修复：restore() Fingerprint seen 分支改 `url.parse::<u64>()` 直接插入
  - commit c8c0328；新测试 fingerprint_seen_roundtrip_preserves_hashes（先 pop 隔离 seen 分支）；206/206 lib 通过
  - 副作用：rustfmt 机械格式化（import 排序、单行 match 展开、Clone impl 展开），无行为影响
- **Minor 待合并后跟进**（reviewer 未标阻塞，记录备查）：
  - #2 RequestCache 公共 API breaking（get/put/invalidate 加 method 参数）→ 加 doc-comment 标注
  - #3 Method→&str 转换重复 4 处 → 抽 `Method::as_str()` 方法（reviewer 推荐本次做，未做）
  - #4 robots "允许全部" 被 is_empty_rules 误判为失败不缓存 → fetch_robots 返回 Option 或加 fetched_ok 字段
  - #5 acquire 步骤 2 未复用刚释放的 Some 槽（非正确性问题）
  - #6 autoscale 字段名与新语义反向（plan 标 breaking 改名延后）
  - #7 robots mock dead_port = port+1 略脆弱（测试 only）
  - #8 xpath_auto/auto_upgrade_check 中毒路径无对称测试（与 css 同质，风险小）

## P1 架构优化（2026-07-23）
- Task 1: complete (commits a211ca7..6f5c44f, review clean) — Method::as_str() DRY 3 处转换
- Task 2: complete (commits 6f5c44f..324b2a9, review clean; 1 Minor: or_insert vs or_insert_with 非阻断，brief 逐字指定)
- Task 3: complete (commits 324b2a9..82b19bd, review clean) — proxy_clients 改 DashMap；3 Minor 全部 brief 既定接受
- Task 4: complete (commits 82b19bd..2524f3a, review clean) — Scheduler seen/heap 锁分离；10 Minor 全部 brief 既定接受
- Task 5: complete (commits 2524f3a..64ecb8f, review clean; brief bug: meta_serde 缺 Deserialize import，最小修复) — SpiderRequest.meta 跨 checkpoint 持久化；2 Minor（backward-compat 回归为 brief 既定；测试覆盖由 brief 逐字指定）
- Task 6: complete — 全量回归 207 lib + 9 集成测试套件全绿；clippy 27 warnings = 基线（Task 2 减 1）；plan 文件 51 checkbox 全标完成

## 累积 Minor 待最终 review triage（P1 阶段）
- Task 3: fetch_page/fetch_page_inner 提为 pub（brief 既定）；同结构体 DashMap 写法不一致（dashmap::DashMap vs DashMap，brief 既定）；慢路径竞态下偶发多余 Client 构建（无正确性问题）
- Task 4: restore 非原子窗口（checkpoint 恢复场景调度器静默，无风险）；restore 无条件清两个 DashSet（未用者本空，开销可忽略）
- Task 5: backward-compat 回归（旧 checkpoint 二进制格式无法读取，brief 既定设计，建议后续加 format version bump 或 migration note）；测试未覆盖空对象/空数组/布尔（brief 逐字指定）

## 多 Spider 重构（2026-07-31）

**Plan:** docs/superpowers/plans/2026-07-31-multi-spider-refactor.md
**Branch:** codex/multi-spider-refactor

- Task 1: complete (commit 8c6357c) — EngineBuilder headers/ua_rotation/cookie_challenge；default_middlewares 不再无条件注入 UA/Cookie；focused tests + cargo check 通过
- Task 2: complete (commit 604b235) — SpiderBuilder 删除 middleware/pipeline；Engine 新增共享 custom_middlewares/pipelines；novel_crawler/cf_bypass 迁移到 Engine 配置；277 lib tests 通过
- Task 3: complete (commit e2c5682) — Request.spider/with_spider；Spider::accepts_callback；ClosureSpider callback 归属；multi_spider_routing_test 通过
- Task 4: complete (commit a6c9519) — Engine::run_many 共享队列 + per-spider until/stats；单 Spider run 退化为 run_many；277 lib + 相关集成测试通过
- Task 5: complete (commit 1b6f84a) — novel_crawler 双模式（handler/spider）+ novel_10_books_test 2/2 通过
- Task 6: complete (commit f47a353) — README/middleware docs 迁移；lib 277 + 10 个集成套件 + cargo check 全绿；分支工作树干净

## 多 Spider 重构最终状态
- 分支：codex/multi-spider-refactor
- 验收：`cargo test --test novel_10_books_test` 2/2 通过（handler/spider 各爬 10 本）
- 说明：当前会话无子代理工具，按 inline 模式逐任务执行并保留验证关卡
- 后续优化（commit 63d549c）：`Page::follow_links`/`follow_links_n` 的 meta 闭包改为 `Fn(&Page, usize, &Node)`，on_links 可在闭包内直接读 page meta；novel_crawler 抽取 home/detail/chapter 共用 handler
- 页面流程声明式化（commit f14d308）：新增 `Page::content_text`、`SpiderBuilder::on_links_n`/`on_content`；novel_crawler 首页/详情用 on_links、章节页用 on_content，去掉了手动 item 组装

## Crate 拆分（2026-07-31）

**Plan:** docs/superpowers/plans/2026-07-31-crate-split-architecture.md
**Branch:** codex/crate-split
**BASE:** f615bed（基线提交，含既有未提交改动）

- Task 0: complete (commits f615bed..f2426c0) — workspace + wisp-core 骨架；基线提交确认
- Task 1: complete (commits f2426c0..1e19e63) — wisp-core 下沉 DTO/错误/配置/工具/编码；ResponseExt trait 解耦 parse/css；FetchMode 移入 core；sanitize_url 移入 core utils，删除 fetcher→crawl 反依赖；顺带修复基线前失效的 auto_mode_test 两断言与 cr_fix_robots_port_test 端口并发冲突（预存问题）
- Task 2: complete (commits 1e19e63..1ca245d) — wisp-storage
- Task 3: complete (commits 1ca245d..4fdc8fb) — wisp-parser（含 ResponseExt）
- Task 4: complete (commits 4fdc8fb..d2a8ff0) — wisp-proxy（含 config_file）
- Task 5: complete (commits d2a8ff0..6c5d156) — wisp-http，移除 fetcher 依赖
- Task 6: complete (commits 6c5d156..599e4e0) — wisp-browser；Page.session/session_id 改 pub（跨 crate 供 fetcher 使用）
- Task 7: complete (commits 599e4e0..4355984) — wisp-stealth
- Task 8: complete (commits 4355984..ed91633) — wisp-fetcher，无 crawl 反依赖
- Task 9: complete (commits ed91633..c19fcf5) — wisp-crawl，批量路径替换后全量测试通过
- Task 10: complete (commits c19fcf5..3ea74fe) — wisp-mcp
- Task 11: complete (commits 3ea74fe..8ae1619) — facade 收尾：删除根 shim、lib.rs 模块 re-export、根依赖收敛、README workspace 结构 + ResponseExt 示例修正
- Task 12: complete (commits 8ae1619..b202a10) — 全量验证 + scripts/check_deps.ps1 依赖方向断言；novel_flow bench 可运行，性能无回归

**最终状态：** `cargo test --workspace --all-targets` 全绿；`scripts/check_deps.ps1` 通过；依赖方向单向：`core <- storage/parser/proxy/http/browser <- stealth <- fetcher <- crawl <- mcp <- facade`

**说明与偏差：**
- 当前会话无子代理工具，按 inline 模式执行并保留每任务验证关卡
- Task 0 骨架 lib.rs 未声明空子模块（避免空文件编译失败），模块声明随 Task 1 文件落地
- Task 1 把 `FetchMode` 一并下沉到 wisp-core（Request.fetch_mode_override 需要），fetcher 继续 re-export
- Response::parse/css/select_one/find_by_text 迁到 wisp-parser::ResponseExt；调用方需 `use wisp::parser::ResponseExt;`
- 基线前已存在的失败：auto_mode_test 泛化断言过期（v2/v1 按设计泛化为 \d+）、cr_fix_robots_port_test 并发端口相撞（串行化修复）、若干 doctest 引用旧 API（delay/obey_robots/Arc 导入）一并修正
- 累积 Minor（后续计划/最终 review 可跟进）：Page.session/session_id 公开字段；response debug 不再打印 body_len；根 dev-dependencies 保留测试所需依赖

## Crate 拆分集成到 arch-refactor master（2026-08-01）

**Branch:** codex/crate-split-v2（基于 master 2cf8b2a）

- 远程 master 新增 26 个提交（CookieJar/fetcher strategies/EngineConfig/feature gates 等），与拆分分支分叉；按用户确认规则合并：冲突文件以 codex/crate-split 为准，master 增量按需移植
- 移植增量：
  - `wisp-fetcher` 引入 master 的 `cookie`（Cookie/CookieJar/Http/Browser/Cf）、`strategy`（BrowserFetchStrategy）、`strategies`（Dynamic/Stealth）
  - `wisp-http` 增加 `cookie_provider` 支持外部共享 jar
  - `wisp-core::error` 增加 `Config` 变体、`StorageError` 细分变体（NotFound/Serialization/Backend/Corrupted）
  - `wisp-parser::adaptive` 增加 ElementSnapshot <-> ElementSnapshotRow `From` 双向转换（放在 parser 侧避免 storage→parser 环）
  - `wisp-crawl::runner` 公开 `EngineConfig` 只读快照 + `Engine::config()`，facade re-export
  - 根 Cargo.toml 增加 http/browser/stealth/mcp features（默认全开），wisp-fetcher 对应 browser/stealth features
  - 删除 master 已清理的 `session_pool`（保持其删除语义）
- 未移植（记录备查）：master 的 async `Store` trait 重构（会牵动 parser/middleware/checkpoint 全链路，独立大改动）；AdaptiveTracker（与 parser::css_adaptive 功能重复）；feature gates 完整下钻（仅默认全开）
- 适配：crawl engine 的 CF 快速路径改用 `cookie_jar().header()`，浏览器路径改用 Dynamic/StealthStrategy；`fetch_browser(req, bool)` 旧签名在测试中更新
- 验证：`cargo test --workspace --all-targets` 全绿；`cargo build --workspace` 无警告
- 基准（master 99818b4，2026-08-01）：novel_flow/multi_spider_10books 中位数 32.928 ms（基线 32.303 ms，无回归）；auto_default 32.9 ms、http_with_transport 20.9 ms、http_minimal 21.5 ms、http_cached_replay 9.8 ms；docs/performance.md 已更新
