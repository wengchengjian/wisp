# Task 7+8 (合并): EngineContext 拆分 + 共享队列路由

这是整个 plan 中最大的重构。Task 7 和 8 紧耦合（EngineContext 变化导致 mod.rs 构造点编译失败），必须一起完成才能编译通过。

## 目标

1. **EngineContext 拆分**（Task 7）：去掉 per-spider 字段（spider/stats_*/allowed/max_depth/fetcher_config/fetch_mode/max_concurrent/obey_robots/rule_engine/auto_exclude/max_pages），改为持有 `Vec<Arc<dyn Spider>>` + `Vec<Arc<SpiderStats>>` + per-spider 配置数组。`process_request`/`process_response`/`fetch_with_retry` 接收 `idx: usize` 参数。

2. **共享队列 + 路由**（Task 8）：`run_with_sender` 改为从共享 scheduler 取 URL → 遍历 spiders 调 `matches()` → 检查 `until().should_stop()` → `process_request(ctx, req, idx)`。删除 `run_spider_once`。

## 完整代码

**读 plan 文件获取完整代码：**
`f:\project\wisp\docs\superpowers\plans\2026-07-22-shared-queue-stop-condition.md`
- 第 759-1403 行包含 Task 7 和 Task 8 的完整代码
- Task 7 在第 759 行开始
- Task 8 在第 1138 行开始

按 plan 中的代码实现，但需要注意以下需要自己适配的部分：

## 需要自己 grep 和适配的辅助函数

plan 中只给了方向，没给完整代码的部分：

1. **`record_status`**：改为接收 `&Arc<SpiderStats>` 而非 `&EngineContext`
2. **`apply_delay`**：改为接收 `spider: &Arc<dyn Spider>` 参数（因为原来从 ctx.spider 拿 download_delay）
3. **`auto_upgrade_check`**：改为接收 `idx` 参数，从 ctx.spiders[idx] 等数组取值
4. **`save_checkpoint`**：改为接收 `&Arc<SpiderStats>` 参数，仅单 Spider 时调用
5. **`snapshot_stats` / `snapshot_stats_for`**：改为从 `&Arc<SpiderStats>` 取值
6. **`InFlightGuard`**：确保为 `pub(crate)` 可见性

用 Grep 搜索这些函数的定义和调用点：
```
grep -n "fn record_status\|fn apply_delay\|fn auto_upgrade_check\|fn save_checkpoint\|fn snapshot_stats\|struct InFlightGuard" src/crawl/engine.rs
```

## 关键约束

- `EngineContext` 保留共享字段（client/sched/robots_cache/follow_tx/follow_rx/domain_sems/proxy_pool/cache_store/request_cache/abort_flag/start/tx/dev_mode）
- 新增 `spiders: Vec<Arc<dyn Spider>>`、`stats: Vec<Arc<SpiderStats>>`、`rule_engines`、`auto_excludes`、`allowed_list`、`fetcher_configs`、`fetch_modes`、`max_concurrents`、`max_depths`、`obey_robots_flags`、`global_in_flight`、`engine_max_pages`
- `process_request(ctx, req, idx)` — idx 是命中的 Spider 下标
- `process_response(ctx, resp, req, idx)`
- `fetch_with_retry(ctx, req, idx)`
- 路由逻辑：`spiders[i].matches(url)` → 检查 `until().should_stop()` → `process_request`
- 无匹配 URL → 丢弃（tracing::debug）
- 引擎退出：共享队列空 + global_in_flight == 0
- `Engine::new(spider)` 和 `Engine::spiders(vec)` 都要兼容

## 验证步骤

1. `cargo build --lib` — 必须通过
2. `cargo test --lib` — 现有测试必须通过
3. `cargo test --test stop_condition_test` — 必须通过
4. 检查是否有遗漏的 `ctx.spider`、`ctx.stats_*`、`ctx.allowed` 等旧字段引用

## Commit

```bash
git add src/crawl/engine.rs src/crawl/mod.rs
git commit -m "refactor: 共享队列 + matches 路由 + until 终止策略" -m "EngineContext 拆分 per-spider 字段，run_with_sender 改为共享队列路由"
```
