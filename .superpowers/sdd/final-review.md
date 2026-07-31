# 阶段 1（P0 硬伤）最终全分支 Code Review

**Reviewer**: sub-agent (GLM-5.2)
**Branch range**: `fc5929a..7e33640`（11 commits，10 个 Task + 1 个 fix round）
**日期**: 2026-07-21
**依据**: spec（阶段 1 章节）、plan、task-8/9/10 review 记录、完整 diff、源码复核

---

## 一、分支概览（10 个 Task 实现摘要）

| # | Commit | Task | 关键产出 | 状态 |
|---|---|---|---|---|
| 1 | `4cdddf1` | 依赖与 error 变体 | Cargo.toml 加 rusqlite/bincode/tokio-stream/chrono；error.rs 加 Storage/AdaptiveError/Serialize/McpError | ✅ |
| 2 | `95b0ac0` | SQLite 统一存储层 | `storage/mod.rs` + `migrations.rs`，SCHEMA_V1 两张表 | ✅ |
| 3 | `58085e8` | difflib SequenceMatcher | `parser/difflib.rs` ~190 行，含 autojunk + find_longest_match + ratio；7 个 Python 对照测试 | ✅ |
| 3a | `fe59692` | difflib fix | 移除死字段 `fullbcount`（review 中发现） | ✅ |
| 4 | `3b58f5f` | ElementSnapshot 捕获 | `parser/adaptive.rs` 重写；`Store::save_element/load_element` + `ElementSnapshotRow` | ✅ |
| 5 | `3365964` | 6 维 similarity + relocate | `similarity()` + `relocate_with_snapshot()`；3 个测试 | ✅ |
| 6 | `629619e` | Node::css_adaptive | 高层 API + 自动快照保存；2 个测试 | ✅ |
| 7 | `602f76e` | Scheduler async + Mutex | `Scheduler` 改为 `Arc<Mutex<Inner>>`，async push/pop/pending_urls/seen_urls/restore | ✅ |
| 8 | `7b431d5` | Engine buffer_unordered | `stream::unfold + buffer_unordered(N)` + per-domain 信号量 + follow channel；1 个 #[ignore] 测试 | ✅ |
| 8a | `0b4e731` | Task 8 fix round | C1 in-flight 跟踪（RAII guard）+ I1 download_delay + I2 robots_cache 注释 + M5 删 unused import | ✅ |
| 9 | `94734bc` | CrawlState + checkpoint | `crawl/state.rs` + `Store::save/load/delete_checkpoint`；Engine 集成恢复/定期保存/清理；4 个测试 | ✅ |
| 10 | `7e33640` | 端到端集成测试 | `tests/integration.rs::adaptive_test` mod，1 个 end-to-end 测试 | ✅ |

**累计代码量**: ~1622 行新增（按 plan 估算），50+ 测试用例（34 lib + 7 difflib + 5 adaptive + 4 checkpoint + 1 integration + 1 ignored）全部通过。

---

## 二、架构评估

### 2.1 整体设计合理性

整体架构与 spec/plan 一致，分层清晰：

```
┌─────────────────────────────────────────┐
│  crawl/Engine (buffer_unordered + C1)  │
│   ├─ Scheduler (async Mutex)            │
│   ├─ CrawlState (bincode blob)         │
│   └─ per-domain Semaphore              │
└──────────────┬──────────────────────────┘
               │ uses
┌──────────────▼──────────────────────────┐
│  storage/Store (rusqlite + SCHEMA_V1)   │
│   ├─ element_snapshots (adaptive)       │
│   └─ crawl_checkpoints (blob)           │
└──────────────┬──────────────────────────┘
               │ used by
┌──────────────▼──────────────────────────┐
│  parser/adaptive                        │
│   ├─ ElementSnapshot (capture/to_row)  │
│   ├─ SequenceMatcher (difflib)         │
│   ├─ similarity (6-dim)                │
│   └─ relocate_with_snapshot            │
└─────────────────────────────────────────┘
```

### 2.2 模块间一致性

**✅ storage::Store API 一致使用**：
- adaptive 通过 `Store::save_element/load_element` + `ElementSnapshotRow` 解耦（storage 层不依赖 parser::Node，正确避免循环依赖）
- checkpoint 通过 `Store::save_checkpoint/load_checkpoint/delete_checkpoint` + raw bytes（同样避免 storage → crawl 依赖）
- 两套 API 风格一致：`Result<T>` + `WispError::Storage` 包装

**✅ parser::Node::css_adaptive 与 Engine 集成**：
- `Node::css_adaptive` 是 parser 层 API，Engine 当前不直接调用
- 用户在 `Spider::parse()` 内部调用 `resp.parse()?.css_adaptive(...)` 即可使用，集成路径正确
- 注意点见 I2（Store 跨并发调用未 Mutex 包装）

**✅ Scheduler async API 被 Engine 正确使用**：
- `sched.push(req).await` / `sched.pop().await` / `sched.pending_urls().await` 全部正确 await
- `Arc<Scheduler>` 在 unfold closure 中 clone 共享，无死锁风险

### 2.3 架构完整性

**✅ adaptive 6 维 similarity 合理**：
- 权重分配 (1.0 + 2.0 + 2.0 + 1.5 + 1.0 + 0.5 = 8.0) 与 spec 一致
- 归一化 `score / max` 正确
- 三 strategy 候选筛选（id → class → tag）是合理的性能优化，与 Python Scrapling 对齐

**✅ Engine buffer_unordered 并发模型正确**（经 C1 fix 后）：
- in-flight 计数用 `Arc<AtomicUsize>` + RAII guard (`InFlightGuard`)，覆盖所有退出路径（含 panic unwind）
- unfold 终止条件三要素 AND：`sched.is_empty() && channel_empty && in_flight == 0`
- yield_now 非 busy loop（依赖 in-flight future 的 await 点驱动唤醒）
- budget 达上限后等 in-flight 完成才退出，与原串行 `break` 语义一致

**✅ checkpoint 流程完整**：
- 启动：load → deserialize → `tracing::info!`（恢复 pending）/ `tracing::warn!`（反序列化失败）
- 运行：每 N 页（默认 100）`pending_urls().await` + `CrawlState::from_stats` + `bincode::serialize` + `save_checkpoint`
- 完成：`on_close()` 后 `delete_checkpoint`，失败 `tracing::warn!`
- 已知简化：seen_urls 不恢复（placeholder）、Ctrl+C 未实现（依赖定期保存兜底）

### 2.4 错误处理

**静默忽略清单**：
- `css_adaptive` 中 `let _ = store.save_element(...)` — best-effort，可接受
- `follow_tx_c.send(f)` 的 `let _` — channel 关闭即 stream 终止，可接受
- `sem.acquire_owned().await.unwrap()` — 信号量未 close，实际不 panic，task-8 review 接受
- 定期 `bincode::serialize` 失败 + `save_checkpoint` 失败 — 累积 Minor M2，见 §五

**错误变体合理性**：
- `Storage(String)` — SQLite 错误，使用充分
- `AdaptiveError(String)` — 当前未使用（relocate_with_snapshot 返回 Option 而非 Result）。可接受，预留未来扩展
- `Serialize(String)` — 当前未使用（bincode/serde 错误都用 `?` 传播或 `unwrap_or_default`）。可接受，MCP 阶段会用到
- `McpError(String)` — 阶段 3 预留，合理

### 2.5 测试覆盖

| 模块 | 测试数 | 覆盖 | 缺口 |
|---|---|---|---|
| difflib | 7 | ident/diff/partial/empty/word/longer | 充分 |
| adaptive | 5 + 1 集成 | capture/relocate/css_adaptive/无快照 | 跨 tag 重定位未覆盖（已知限制） |
| checkpoint | 4 | save/load round-trip/delete/missing/defaults | 充分 |
| Engine run | 1 (#[ignore]) | smoke test | 无离线并发测试（M4） |
| Scheduler | 0（仅 lib 单元测试） | — | push/pop dedup/restore 未覆盖 |

**核心路径覆盖足够**，Engine 并发正确性靠代码审查（RAII guard）保证，可接受 stage 1 简化。

### 2.6 代码风格

**Dead code / unused**（cargo check --tests 输出）：
- `tests/crawl_checkpoint_test.rs:3` `use std::sync::Arc;` 未使用（累积 Minor）
- `tests/adaptive_test.rs` 有 `unused variable: store`（line 58, test_relocate_returns_none_when_no_match）
- `src/storage/mod.rs:45` `Store::conn()` pub(crate) accessor 未使用（新发现 I3）
- 5 个预先存在的 lib warning（headless/wait_js_challenge/wait_managed/CommandExt/opts），非本分支引入

**命名一致性**：
- `EngineConfig.max_concurrent` vs `Spider::concurrent_requests()` — 语义一致，命名略不同，可接受
- `with_checkpoint(store: Arc<Store>)` vs `Store::save_checkpoint` — 一致

**注释清晰度**：
- C1 fix 注释详尽（RAII guard 原理 + 所有退出路径覆盖说明）
- I2 fix 注释清晰（问题/影响/接受/改进方向）
- **但 I1 的注释有事实错误**（见下文 I1）

---

## 三、Findings（新发现，不重复已知 stage 1 简化）

### Critical

（无）

### Important

**I1（跨 task）: `SpiderRequest.meta` 的代码注释事实性错误**

- **位置**: `src/crawl/mod.rs:35-37`
- **现状**:
  ```rust
  // `serde_json::Value` 的 Deserialize 依赖 `deserialize_any`，bincode 不支持。
  // checkpoint 场景下 `meta` 当前不被读取，跳过它以让 bincode round-trip 可行。
  // JSON 序列化语义不受影响（仅 bincode 路径跳过）。
  #[serde(skip)]
  pub meta: Value,
  ```
- **问题**: 第三行注释**事实错误**。`#[serde(skip)]` 是 serde 通用属性，**对所有 Serializer 生效**，包括 `serde_json::Serializer` 和 `bincode::Serializer`。若未来用 `serde_json::to_string(&spider_request)`，`meta` 字段会被**跳过**（序列化结果不含 `meta` 键），并非注释声称的"JSON 序列化语义不受影响"。
- **实际影响**: 当前代码库无 serde_json 序列化 SpiderRequest 的场景，故无功能 bug。但注释会误导未来 maintainer 误判 `#[serde(skip)]` 的影响范围，可能在 stage 2/3 引入 JSON 序列化 SpiderRequest 时踩坑。
- **来源**: task-9-review.md 的 I1 已指出此问题（report 声明错误），但**代码注释本身同样有错误，未被纠正**。
- **修复**: 1 行注释修改，零代码变更。建议改为：
  ```rust
  // `serde_json::Value` 的 Deserialize 依赖 `deserialize_any`，bincode 不支持。
  // checkpoint 场景下 `meta` 当前不被读取，跳过它以让 bincode round-trip 可行。
  // 注意：`#[serde(skip)]` 对所有 Serializer 生效（含 serde_json），未来若用
  // serde_json 序列化 SpiderRequest 需重新评估（改用 `#[serde(with = "...")]`）。
  ```

**I2（跨 task）: `Store` 在 Engine 中以 `Arc<Store>` 共享但未 Mutex 包装，与 Store 文档不一致**

- **位置**: `src/crawl/mod.rs:128`（`checkpoint_store: Option<Arc<crate::storage::Store>>`）、`src/storage/mod.rs:14-15`（doc comment）
- **现状**: Store 文档注释：
  ```rust
  /// Unified SQLite store. Inner connection is NOT thread-safe by itself;
  /// callers wrap it in `Arc<Mutex<Store>>` for concurrent access.
  ```
  但 Engine 中是 `Option<Arc<Store>>`，**没有 Mutex**。
- **当前安全性分析**: 目前 `checkpoint_store` 只在 `Engine::run` 的主循环中被调用（`load_checkpoint` 在 run 开头、`save_checkpoint` 在 stream.next() 之间、`delete_checkpoint` 在 on_close 后）。这些调用点都在主 task 中，不在 `buffer_unordered` 的并发 future 内。所以**当前无数据竞争**。
- **风险**: 模式脆弱。若用户在 `Spider::parse()` 内部调用 `store.save_element()`（adaptive 场景的合理用例），而 parse 在 `fut` async block 内被 buffer_unordered 并发执行，会触发 SQLite connection 的数据竞争。文档与实际用法不一致，未来扩展易踩坑。
- **修复建议**（任选其一）:
  1. 把 `checkpoint_store` 改为 `Arc<Mutex<Store>>`，所有调用加 `.lock().await`
  2. 更新 Store 文档：「单 task 内访问无需 Mutex；多 task 并发访问需 `Arc<Mutex<Store>>`」
  3. 推迟到 stage 2（adaptive 集成 Engine 时一并处理）
- **建议**: stage 1 接受，但**至少更新文档**（选项 2，零代码变更）

**I3（跨 task）: `Store::conn()` pub(crate) accessor 是 dead code**

- **位置**: `src/storage/mod.rs:45-48`
- **现状**:
  ```rust
  /// Raw connection accessor (for module-internal queries).
  pub(crate) fn conn(&self) -> &Connection {
      &self.conn
  }
  ```
  编译器 warning：`method 'conn' is never used`。全库 grep 无调用。
- **来源**: plan Task 2 Step 2 加了"为模块内部查询预留"，但实际从未使用。
- **修复**: 删除该方法和注释（4 行删除），消除 warning。

### Minor

**M1（跨 task）: `Scheduler::restore()` 和 `Scheduler::seen_urls()` 是 dead code，且返回 placeholder**

- **位置**: `src/crawl/scheduler.rs:86-94`（`seen_urls`）、`105-124`（`restore`）
- **现状**:
  - `seen_urls()` 返回 `HashSet<String>`，但实际是 `h.to_string()`（hash 的字符串形式），不是真实 URL — **API 误导**
  - `restore()` signature 完美匹配 checkpoint 恢复，但 Task 9 用 `push()` 循环替代，从未调用
  - 编译器未发 warning 因 `Scheduler` 是 `pub`，方法也是 `pub`（外部可见，编译器不报 dead code）
- **影响**: API 表面有"功能完整"的假象，实际未使用且不可用（`seen_urls` 返回的数据无意义）
- **修复建议**: 
  - `seen_urls()`: 改为 `pub async fn seen_urls(&self) -> HashSet<String>` 内 `unimplemented!("stage 1 placeholder")` 或直接删除并在 trait 加 TODO 注释
  - `restore()`: 删除或加 `#[allow(dead_code)]` + TODO 注释说明 stage 2 启用
- **紧迫性**: 低，不阻塞，但建议 stage 1 cleanup 时处理

**M2（跨 task）: `ElementSnapshot::position_in_parent` 持久化但从未被 similarity 使用**

- **位置**: `src/parser/adaptive.rs:25`（field）、`117`（capture）、`148`（to_row）、`167`（from_row）
- **现状**: 字段被捕获、序列化、存 SQLite、反序列化，但 `similarity()` 函数 6 维评分中**不包含** position_in_parent（按 spec 1.1.4 设计正确）
- **影响**: 存储 + 序列化成本但无功能价值。Spec 明确不要求，但代码里"看着像在用"
- **建议**: 在字段注释中加 `// reserved for future use (not in stage 1 similarity scoring)`，或 stage 2 similarity 扩展时启用

**M3（跨 task）: `Store` 有两个 `impl` 块，人为分割**

- **位置**: `src/storage/mod.rs:20-85`（checkpoint 方法）、`101-160`（element 方法 + `ElementSnapshotRow` 在中间）
- **现状**: 风格上 unusual（虽然 Rust 合法），可能是 Task 2 创建第一个 impl，Task 4 追加第二个
- **影响**: 可读性略降低，无功能影响
- **建议**: 合并为单个 `impl Store` 块（可选，零功能变更）

**M4（跨 task）: adaptive 的 helper 函数每次 similarity 调用都重复解析 outer_html 4 次**

- **位置**: `src/parser/adaptive.rs:341-419`（`node_tag_name` / `ancestor_path_of` / `sibling_tags_of` / `parent_attrs_of`）
- **现状**: 每个函数都执行 `format!("<html><body>{}</body></html>", outer)` + `Html::parse_document(&full)`，单次 `similarity()` 调用 = 4 次解析
- **影响**: 性能损失（4× 重复解析）。已知 stage 1 限制，spec 1.1.2 明确说"stage 1 临时实现，stage 2 用 Node::ancestors() 替换"
- **建议**: stage 1 接受，stage 2 Node 重构后修复（不阻塞）

**M5（跨 task）: `Engine::run` 中 `domain.clone()` 可省略**

- **位置**: `src/crawl/mod.rs:346`
- **现状**: `sems.entry(domain.clone())` 中 `domain` 后续未使用，可改为 `sems.entry(domain)` 移动所有权
- **影响**: 微观优化（省一次 String clone）
- **建议**: 可选修复，1 行改动

---

## 四、累积 Minor findings triage

### 来自各 task review 的累积 Minor

| # | 来源 | Finding | 处理建议 | 理由 |
|---|---|---|---|---|
| 1 | task-9 review M1 | `tests/crawl_checkpoint_test.rs:3` `use std::sync::Arc;` 未使用 | **Fix now** | 1 行删除，消除 warning，零风险 |
| 2 | task-8 review M5 | `tests/crawl_concurrency_test.rs:3` 未使用 Arc | **Already fixed**（Task 8 fix round 已删） | 已完成 |
| 3 | task-9 review M2 | 定期 checkpoint 保存失败静默忽略 | **Fix now** | 加 `tracing::warn!` 成本极低（~5 行），诊断价值高，且与已有的 `delete_checkpoint` 失败 `warn!` / `deserialize` 失败 `warn!` 保持一致 |
| 4 | task-8 review M1 | per-domain 信号量许可数 = max_concurrent | **Defer to stage 2** | per brief 设计，需新增 `per_domain_concurrent` 配置项 |
| 5 | task-8 review M2 | page budget 是软限制 | **Accept** | 并发常见权衡，最多超出 `max_concurrent - 1` 个 |
| 6 | task-8 review M3 | `sem.acquire_owned().await.unwrap()` | **Accept** | 实际不会 panic（信号量未 close） |
| 7 | task-8 review M4 | `crawl_concurrency_test` 标记 `#[ignore]` | **Defer to stage 2/3** | 需引入 wiremock 或本地 HTTP server，stage 1 不阻塞 |
| 8 | task-8 review M6 | `domain.clone()` 可省略 | **Fix now** | 1 行改动，零风险（与本文 M5 合并修复） |
| 9 | task-9 review M3 | `Scheduler::restore` 未被调用 | **Fix now**（与本文 M1 合并） | dead code，应删除或加 TODO |

### 推荐的 cleanup commit 范围

若选择做 cleanup commit，建议包含以下零风险改动：

1. 删除 `tests/crawl_checkpoint_test.rs:3` 的 `use std::sync::Arc;`（累积 #1）
2. 删除 `src/storage/mod.rs:45-48` 的 `Store::conn()` 方法（本文 I3）
3. 删除或 TODO 标注 `src/crawl/scheduler.rs` 的 `seen_urls()` 和 `restore()`（本文 M1 + 累积 #9）
4. 修正 `src/crawl/mod.rs:35-37` 的注释错误（本文 I1）
5. 加 `tracing::warn!` 给定期 checkpoint 保存失败路径（累积 #3）
6. `src/crawl/mod.rs:346` 改 `sems.entry(domain)` 省略 clone（累积 #8 / 本文 M5）
7. 更新 `src/storage/mod.rs:14-15` 的 Store 文档说明单 task vs 多 task 用法（本文 I2）

总改动量：~30 行，零功能变更，消除 3 个 warning，提高代码可信度。

---

## 五、总评

### **APPROVED**

### 评分理由

**实现完整性**: ✅
- 10 个 Task 全部完成，11 个 commit（含 1 个 fix round）
- 所有 spec 阶段 1 章节要求满足（adaptive 完整移植 + Spider 并发 + checkpoint）
- 50+ 测试全部通过（除 1 个 #[ignore] 网络测试）
- C1 Critical fix（in-flight 跟踪 + RAII guard）设计稳健，覆盖所有退出路径

**架构合理性**: ✅
- 模块分层清晰，storage/parser/crawl 职责边界正确
- 无循环依赖（storage 用 raw bytes / Row 类型解耦）
- Scheduler async API + Engine buffer_unordered 集成正确
- Checkpoint 恢复/保存/清理流程完整

**代码质量**: ⚠️ → ✅（带 cleanup 建议）
- 3 个新 Important findings（I1 注释错误 / I2 Store 共享模式 / I3 dead code）均为**文档/设计层面问题**，非功能 bug
- 5 个新 Minor findings 均为风格/优化建议
- 累积 Minor 中 4 项建议 fix now（零风险），其余 defer/accept

**已知简化合规**: ✅
- 所有 stage 1 简化（seen_urls placeholder / Ctrl+C 未实现 / robots_cache 锁跨 await / per-domain 信号量 / page budget 软限制 / crawl_concurrency_test #[ignore]）均在 plan/task review 中记录在案，不在本 review 重复

### 不阻塞合并

本分支可合并到 main。建议在合并前或合并后立即做一次 cleanup commit（见 §四推荐范围），消除 3 个 warning 并修正 1 个事实错误的注释。

### Stage 2 需关注的遗留项

1. `Store` 共享模式（I2）— 若 stage 2 adaptive 集成 Engine，必须解决
2. `Scheduler::seen_urls` 真实化 — stage 2 应追踪原始 URL 而非 hash
3. `Scheduler::restore` 启用 — stage 2 改用 `restore()` 替代 `push()` 循环
4. `ElementSnapshot::capture` 重写 — stage 2 用 `Node::ancestors()/parent()` 替代 outer_html 重复解析
5. 离线并发测试 — stage 2/3 用 wiremock 补 Engine run 的并发正确性测试

---

## 附录：审查方法论

1. 读取完整 spec / plan / 3 个 task review 记录
2. 通过 `git log fc5929a..HEAD` 确认 11 个 commit
3. 直接读取关键源文件复核：`crawl/mod.rs` / `crawl/scheduler.rs` / `crawl/state.rs` / `storage/mod.rs` / `parser/adaptive.rs` / `parser/difflib.rs` / `parser/mod.rs` / `error.rs` / `lib.rs`
4. 运行 `cargo check --tests` 验证编译 + warning 列表
5. 交叉对比 task review 中的 findings 与当前源码，确认哪些已修复、哪些遗留
6. 跨 task 一致性检查：API 调用链 / 类型一致性 / 错误处理一致性 / 命名一致性
7. 不重复已知 stage 1 简化（已在 task description 中明确列出）
