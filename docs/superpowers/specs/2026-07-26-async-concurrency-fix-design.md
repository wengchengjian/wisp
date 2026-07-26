# 异步与并发性能修复设计

> 基于 `docs/async-concurrency-review.md` 审查报告，修复 11 项关键异步/并发性能问题。
> 设计日期：2026-07-26
> 实施分支：master（直接开发，禁止 worktree/feature branch）

---

## 1. 背景与目标

wisp 框架在异步与并发层面存在多处性能反模式：存储层同步 I/O 阻塞 async runtime、CDP 事件双重存储与锁争用、items 收集每项抢锁、follow_rx 不必要的 Mutex 包装等。审查报告（`docs/async-concurrency-review.md`）共识别 22 项问题，本设计聚焦其中 11 项高价值修复。

**目标**：
- 消除 async runtime 阻塞点
- 降低锁争用与锁粒度
- 提升高并发吞吐与 P99 延迟
- 保持现有 273 个测试全过

**非目标**：
- 不引入 criterion 基准测试（避免新增 dev-dependency）
- 不重构 Scheduler 为 lock-free（M9，架构改动过大）
- 不重构 SessionPool 为 BinaryHeap（M5，非核心路径）
- 不处理 Low 级别细节优化（L1-L7，后续 PR）

---

## 2. 修复范围

### High（5 项）
| 编号 | 问题 | 文件 |
|---|---|---|
| H1 | SqliteStore 同步 I/O 阻塞 async runtime | `storage/sqlite.rs` |
| H2 | FileStore 同步 I/O + 全局写锁 | `storage/file.rs` |
| H3 | CdpSession 事件 Vec + consumed_offset 双锁争用 | `browser/cdp.rs` |
| H4 | process_response 每 item 抢锁 + 阻塞 send | `crawl/engine.rs` |
| H5 | follow_rx Mutex 包装单消费者 Receiver | `crawl/engine.rs`、`crawl/runner.rs` |

### 关键 Medium（6 项）
| 编号 | 问题 | 文件 |
|---|---|---|
| M1 | fetch_dispatch 重试无退避抖动 | `crawl/engine.rs` |
| M2 | persist_spider_checkpoint 主循环同步执行 | `crawl/runner.rs` |
| M3 | EventBus.emit 串行执行 listener | `crawl/observability/events.rs` |
| M4 | fetch_dispatch AutoFallback rule_engine 双重锁 | `crawl/engine.rs` |
| M6 | CDP 后台任务错误未通知 pending | `browser/cdp.rs` |
| M7 | tx.send().await 阻塞核心路径 | `crawl/engine.rs` |

---

## 3. 整体方案

**方案 A：分 Phase 渐进式**（已选）

6 个独立 commit，每个 Phase 内部 TDD（红→绿→重构）。每 Phase 完成后跑全量测试，绿则进入下一 Phase。风险隔离强：单个 Phase 出问题可独立 revert。

### Phase 依赖关系

```
Phase 1: CdpSession 重构 (H3 + M6)         [独立模块]
    ↓
Phase 2: items 批量 + try_send (H4 + M7)   [engine 内部]
    ↓
Phase 3: follow_rx Mutex 移除 (H5)         [runner 内部]
    ↓
Phase 4: fetch_dispatch 优化 (M1 + M4)     [engine 内部]
    ↓
Phase 5: checkpoint + EventBus (M2 + M3)   [runner + observability]
    ↓
Phase 6: Store trait async 化 (H1 + H2)    [影响面最大，最后做]
```

### 各 Phase 概要

| Phase | 修复项 | 改动文件 | 预估行数 | 破坏性 |
|---|---|---|---|---|
| 1 | H3 + M6 | `browser/cdp.rs` | ~200 | 中（删除字段） |
| 2 | H4 + M7 | `crawl/engine.rs` | ~60 | 低（内部重构） |
| 3 | H5 | `crawl/engine.rs`、`crawl/runner.rs`、`crawl/builder.rs` | ~80 | 中（结构体字段调整） |
| 4 | M1 + M4 | `crawl/engine.rs`、`crawl/middleware/builtin.rs` | ~50 | 低 |
| 5 | M2 + M3 | `crawl/engine.rs`、`crawl/runner.rs`、`crawl/observability/events.rs` | ~70 | 低 |
| 6 | H1 + H2 | `storage/{mod,sqlite,file,memory}.rs` + 调用点 8 个 | ~300 | 高（trait 签名变更） |

### 关键约束

1. **master 分支直接开发**，每 Phase 一个 commit（中文简短信息）
2. **不向后兼容**：删除旧字段/方法，不保留兼容层
3. **TDD**：每 Phase 先写失败测试，再实现
4. **现有 273 测试必须全过**：每 Phase 完成后 `cargo test --all-features`

---

## 4. 各 Phase 详细设计

### Phase 1 — CdpSession 重构（H3 + M6）

**改动文件**：`browser/cdp.rs`

**关键变更**：
1. 删除字段 `events: Arc<Mutex<Vec<CdpEvent>>>` 和 `consumed_offset: Arc<Mutex<usize>>`
2. 新增字段 `conn_state: tokio::sync::watch::Sender<ConnState>`
3. 新增枚举 `ConnState { Open, Closed(String) }`
4. `connect` 后台任务：错误时 `conn_state.send(Closed(...))` + 清空 pending，避免 30s timeout 等待
5. `execute_with_session`：注册 pending 前检查 conn_state；`select!` 同时等待 oneshot 响应和 conn_state 变化
6. `wait_for_event`：改用 `broadcast::Receiver::recv()` + `select!` timeout，删除 Vec 扫描

**调用点影响**：`subscribe_events()` 签名不变（`fetcher/client.rs:332`、`browser/page.rs:441` 无需改）

**TDD 测试**：
- `test_cdp_event_broadcast_no_vec`：验证事件经 broadcast 直达订阅者
- `test_cdp_connection_error_notifies_pending`：模拟 ws 错误，验证 execute 立即返回而非等 30s
- `test_wait_for_event_uses_broadcast`：验证 lagged 消费者不卡死

---

### Phase 2 — items 批量 + try_send（H4 + M7）

**改动文件**：`crawl/engine.rs`

**关键变更**：
1. `process_response` items 段（L289-316）：本地 `Vec::with_capacity(items.len())` 收集，单次 `items.lock().await.extend(...)` 批量 push
2. `engine.rs:307` `tx.send(CrawlEvent::Item).await` → `tx.try_send(...)`，channel 满时 `tracing::warn` 并丢弃
3. `engine.rs:166-172` 错误事件 `tx.send().await` → `try_send`
4. 保留 `runner.rs:134, 143` 的 `tx.send(Done).await`（流结束低频事件，必须保证送达）

**TDD 测试**：
- `test_items_batch_push_single_lock`：100 items 验证只抢一次锁
- `test_try_send_drops_on_full_channel`：填满 channel 后验证 try_send 不阻塞

---

### Phase 3 — follow_rx Mutex 移除（H5）

**改动文件**：`crawl/engine.rs`、`crawl/runner.rs`、`crawl/builder.rs`

**关键变更**：
1. `EngineShared` 删除字段 `follow_rx: Arc<Mutex<UnboundedReceiver<Request>>>`
2. `runner.rs` 的 `stream::unfold` 状态中持有 `UnboundedReceiver`（move 进闭包）
3. unfold 内 `rx.try_recv()` 循环 drain，无需锁
4. engine.rs 4 处测试构造（L839, L950, L1061, L1253）+ runner.rs 主构造（L228）移除 `Arc::new(Mutex::new(...))` 包装
5. 测试中需要单独获取 follow_rx 的，改为从 runner 返回值或测试 helper 暴露

**受影响测试构造点**（全部需调整）：
- `engine.rs:839` — `Engine::run` 测试 helper
- `engine.rs:950` — retry 测试
- `engine.rs:1061` — refetch 测试
- `engine.rs:1253` — 其他 engine 测试
- `runner.rs:228` — `run_inner` 主构造

**TDD 测试**：
- `test_follow_rx_drained_without_mutex`：验证主循环 drain follow 请求无锁争用
- 现有 follow 相关测试调整构造方式

---

### Phase 4 — fetch_dispatch 退避 + rule_engine 单次锁（M1 + M4）

**改动文件**：`crawl/engine.rs`、`crawl/middleware/builtin.rs`

**关键变更**：
1. `engine.rs:386-396` AutoFallback：合并 resolve+learn 到单次 `rule_engine.lock().await`
2. `engine.rs:417-437` Retry 路径：指数退避 `100ms * 2^attempt`，封顶 30s，加 `[0, exp/2)` 抖动
3. `builtin.rs:96-104` `RetryMiddleware::process_error`：移除内部 `tokio::sleep(retry_delay)`（退避由 engine 统一负责，中间件仅决定是否重试）
4. `RetryMiddleware::new` 直接删除 `retry_delay` 参数（不向后兼容）

**退避公式**：
```
delay = min(100ms * 2^attempt, 30s) + rand(0..exp_delay/2)
```

**TDD 测试**：
- `test_rule_engine_single_lock_for_autofallback`：验证 resolve+learn 在单次锁内
- `test_retry_exponential_backoff`：验证 attempt=1→100ms, attempt=2→200ms, attempt=3→400ms
- `test_retry_jitter_range`：验证抖动在 `[0, exp/2)` 范围

---

### Phase 5 — checkpoint offload + EventBus 并发（M2 + M3）

**改动文件**：`crawl/engine.rs`、`crawl/runner.rs`、`crawl/observability/events.rs`

**关键变更**：
1. `events.rs:120-127` `EventBus::emit`：`FuturesUnordered` 并发 await 所有 listener
2. `runner.rs:470-490` checkpoint：`tokio::spawn` 后台执行，主循环不等待；失败仅 `tracing::warn`
3. `engine.rs persist_spider_checkpoint`：保持 async 函数签名（内部 Store 调用在 Phase 6 改 async 后加 `.await`），但调用方不 await
4. 优雅退出：runner 结束时 `tokio::task::yield_now()` 一次让 checkpoint 完成（best-effort，不强制等待）

**TDD 测试**：
- `test_event_bus_concurrent_listeners`：3 个 listener（含 1 个慢），验证总延迟 ≈ max 而非 sum
- `test_checkpoint_spawned_not_blocking_main_loop`：验证主循环在 checkpoint 期间继续派发

---

### Phase 6 — Store trait async 化（H1 + H2）

**改动文件**：`storage/{mod,sqlite,file,memory}.rs` + 调用点 8 个

**关键变更**：
1. `storage/mod.rs` `Store` trait 加 `#[async_trait]`，所有方法改 `async fn`
2. `sqlite.rs` 所有方法：`Arc::clone(&conn)` + `tokio::task::spawn_blocking` 包装同步 I/O
3. `file.rs` 同上模式
4. `memory.rs` moka 非阻塞，直接 `async fn` 包装（不 spawn_blocking）
5. 调用点加 `.await`：
   - `crawl/middleware/builtin.rs`（CacheMiddleware、RobotsMiddleware）
   - `mcp/mod.rs`、`mcp/tools.rs`
   - `parser/adaptive.rs`
   - `crawl/engine.rs`（persist_spider_checkpoint 已在 Phase 5 改）
6. `CachedResponse` 相关 trait/impl 若有同步 Store bound，一并调整

**调用点清单**（全部需加 `.await`）：
| 文件 | 模块 | 调用 |
|---|---|---|
| `crawl/middleware/builtin.rs` | CacheMiddleware | `store.get` / `store.set` |
| `crawl/middleware/builtin.rs` | RobotsMiddleware | `store.get` / `store.set` |
| `mcp/mod.rs` | MCP server | `store.get` / `store.set` / `store.delete` |
| `mcp/tools.rs` | MCP tools | `store.get` / `store.set` |
| `parser/adaptive.rs` | Adaptive parser | `store.get` / `store.set` |
| `crawl/engine.rs` | persist_spider_checkpoint | `store.set`（Phase 6 加 `.await`，函数已是 async） |

**TDD 测试**：
- `test_sqlite_store_async_does_not_block_runtime`：在 async 上下文调用 set，同时验证其他 task 不被阻塞
- `test_file_store_async_concurrent_writes`：并发写不同 namespace，验证不串行化
- 现有 storage 测试改为 `.await`

---

## 5. 测试策略

### TDD 流程（每 Phase 内部）

```
1. 写失败测试（红）        → cargo test <new_test> 失败
2. 实现修复（绿）          → cargo test <new_test> 通过
3. 全量回归（验证）        → cargo test --all-features 全过
4. 重构清理（可选）        → cargo clippy -- -D warnings
5. 提交 commit            → git commit -m "..."
```

### 测试分类

| 类型 | 目的 | 命令 | 频率 |
|---|---|---|---|
| 新增单元测试 | 验证修复点行为 | `cargo test <test_name>` | 每 Phase |
| 全量功能回归 | 确保不破坏现有 273 测试 | `cargo test --all-features` | 每 Phase 完成时 |
| Clippy lint | 代码质量 | `cargo clippy --all-targets --all-features -- -D warnings` | 每 Phase |
| 文档测试 | doc 示例正确 | `cargo test --doc` | 末次 |

### 新增测试清单

| Phase | 新增测试 | 数量 |
|---|---|---|
| 1 | broadcast 直达、错误传播、wait_for_event | 3 |
| 2 | 批量 push 单锁、try_send 满不阻塞 | 2 |
| 3 | follow_rx drain 无锁 + 调整现有 follow 测试 | 1+ |
| 4 | 单次锁、指数退避、抖动范围 | 3 |
| 5 | 并发 listener、checkpoint 不阻塞 | 2 |
| 6 | SQLite async 不阻塞、FileStore 并发写 + 改造现有 storage 测试 | 2+ |

**总新增测试数**：约 13 个新测试 + 改造若干现有测试

---

## 6. 风险与回滚

### 各 Phase 风险

| Phase | 主要风险 | 缓解措施 | 回滚策略 |
|---|---|---|---|
| 1 | CdpSession 字段删除导致编译错误 | 调用点已验证（仅 subscribe_events，签名不变） | `git revert <commit>` |
| 2 | items 批量后顺序语义变化 | 本地 Vec 保留插入顺序，extend 顺序一致 | revert |
| 3 | EngineShared 结构调整影响测试构造 | spec 列出所有 4+1 处构造点，逐一改造 | revert + 恢复测试构造 |
| 4 | 退避延迟过长影响吞吐 | 封顶 30s，attempt 上限 max_retries | revert |
| 5 | checkpoint spawn 后丢失未完成状态 | best-effort 设计，失败仅 warn；退出时 yield_now | revert |
| 6 | Store trait async 化波及 8 文件 | spec 列出所有调用点；MemoryStore 直接 async 无 spawn | 分两步提交：先 trait 改造，再调用点迁移 |

### 全局风险

- **测试覆盖不足**：若现有 273 测试未覆盖某边缘场景，Phase 改动可能引入隐性 bug。缓解：每 Phase 后手动跑一次 banzhu-rs 集成验证
- **Phase 间累积复杂度**：Phase 6 依赖前 5 Phase 的稳定接口。缓解：每 Phase 完成后打 tag（本地），便于回溯

### 回滚原则

- 单 Phase revert 不影响其他 Phase（除 Phase 6 依赖前序接口稳定）
- Phase 6 若失败，可单独 revert trait 改造，保留前 5 Phase 收益

---

## 7. 提交规范

### 分支策略

直接在 `master` 主分支开发（CLAUDE.md 硬约束，禁止 worktree/feature branch）

### Commit message 格式

中文简短一行（符合 CLAUDE.md "Git 提交信息要简短，一行足以"）：

```
perf(cdp): 删除 events Vec 改用 broadcast + 错误传播 watch
perf(engine): items 批量收集 + 事件 try_send 非阻塞
perf(runner): follow_rx 移入 unfold 状态移除 Mutex
perf(engine): fetch_dispatch 退避抖动 + rule_engine 单次锁
perf(observability): EventBus 并发 listener + checkpoint spawn 后台
perf(storage): Store trait async 化 + spawn_blocking 包装
```

### 提交粒度

每 Phase 一个 commit，不拆分（保证 revert 原子性）

### 验证命令（每 Phase 完成前必须全过）

```bash
cd /home/weng/wisp
cargo test --all-features                              # 全量功能测试
cargo clippy --all-targets --all-features -- -D warnings  # lint
```

### Phase 完成标志

1. 新增测试通过
2. 现有 273 测试全过
3. clippy 无 warning
4. commit 已提交

---

## 8. 实施顺序总览

```
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6
 ↓          ↓          ↓          ↓          ↓          ↓
commit1   commit2   commit3   commit4   commit5   commit6
 ↓          ↓          ↓          ↓          ↓          ↓
test+3    test+2    test+1+    test+3    test+2    test+2+
 ↓          ↓          ↓          ↓          ↓          ↓
全量回归   全量回归   全量回归   全量回归   全量回归   全量回归
```

**预估总改动**：~760 行（含测试），6 个 commit

---

## 9. 验收标准

完成全部 6 Phase 后，以下条件必须满足：

1. `cargo test --all-features` 全过（含新增 13+ 测试）
2. `cargo clippy --all-targets --all-features -- -D warnings` 无 warning
3. banzhu-rs 集成验证：使用 wisp path 依赖跑一次真实爬取，确认无回归
4. 6 个 commit 已提交到 master，commit message 符合规范

### 预期收益（对照审查报告）

| 优化项 | 基线 | 目标 |
|---|---|---|
| CdpSession 事件延迟 | 当前 Vec+Mutex | -70%（broadcast 无锁） |
| SqliteStore 阻塞 | 当前同步 I/O | 0 阻塞（spawn_blocking） |
| items 高负载吞吐 | 当前 N 次锁 | +40%（单次锁批量） |
| 重试风暴 | 当前无退避 | -80%（指数退避+抖动） |
| checkpoint 期间主循环 | 当前阻塞 | 不阻塞（spawn 后台） |

---

## 10. 约束遵守

- **不引入新依赖**：仅使用 `tokio`/`futures`/`async-trait`/`parking_lot` 生态（`async-trait` 已在项目中使用）
- **master 分支直接开发**：禁止 worktree/feature branch
- **不向后兼容**：删除旧字段/方法，不保留兼容层
- **TDD**：每 Phase 先写失败测试
- **精准修改**：每行改动可追溯到本设计文档的某项修复

---

## 附录 A：相关文档

- 审查报告：`docs/async-concurrency-review.md`
- 项目规则：`CLAUDE.md`
- 存储 feature 设计：`docs/superpowers/specs/2026-07-25-storage-feature-flag-design.md`
- Tracing 设计：`docs/superpowers/specs/2026-07-24-tracing-instrumentation-design.md`

## 附录 B：决策记录

| 决策 | 选项 | 理由 |
|---|---|---|
| 修复范围 | High + 关键 Medium（11 项） | 平衡收益与工作量 |
| Store 异步化方式 | trait 全量 async | 接口语义统一，避免未来误用 |
| 实现顺序 | 6 Phase 从独立到核心 | 风险隔离，最后做大改动 |
| 验证策略 | TDD + 现有测试全过 | 符合 CLAUDE.md，不引入 criterion |
