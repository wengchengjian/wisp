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
