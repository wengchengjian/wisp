# SDD Progress Ledger

## Round 2: turso-clippy-remaining-perf (2026-07-26)

## Task R2-1: 删除死代码 SessionPool
- **Status**: complete | **Commits**: 80fa97d..f1bb8d1 | **Review**: N/A (纯删除，无代码逻辑)

## Task R2-2: M10 println! 改 tracing::trace!
- **Status**: complete | **Commits**: f1bb8d1..9d3422c | **Review**: N/A (mechanical)

## Task R2-3: M9 Scheduler parking_lot::Mutex
- **Status**: complete | **Commits**: 9d3422c..7f2211a | **Review**: N/A (mechanical)

## Task R2-4: M8 score_body spawn_blocking
- **Status**: complete | **Commits**: 7f2211a..0a1db5c | **Review**: N/A (mechanical)

## Task R2-5: L1/L2/L3/L4/L7 五项小优化
- **Status**: complete | **Commits**: 0a1db5c..63c8635 | **Review**: N/A (mechanical, controller-verified)
- **内容**: UA static str / Proxy Arc<str> / cf_locks moka / 短路扫描 / CDP buffer
- **修复**: cf_domain_locks 在 runner.rs 测试中改用 moka::sync::Cache；ProxyInjectionMiddleware 调用点 Arc<str>→String

## Task R2-6: L5 EventListener Arc<EngineEvent>
- **Status**: complete | **Commits**: 63c8635..feba14b | **Review**: ✅ Approved
- **内容**: EventListener 改 Arc<dyn Fn(Arc<EngineEvent>)>，emit 用 Arc::new + Arc::clone 共享无 clone

## Task R2-7: Turso 替换 rusqlite
- **Status**: complete | **Commits**: feba14b..04978fa | **Review**: ✅ Approved
- **内容**: rusqlite → turso 0.7.0-pre.18，SqliteStore::open/open_in_memory 改 async，Store trait 原生 async 无 spawn_blocking
- **偏差**: PRAGMA journal_mode=WAL 用 query 替代 execute_batch（turso API 限制，已注释说明）
- **Minor findings**:
  - cast_possible_wrap clippy 警告（sqlite.rs:139，pre-existing 非回归）
  - turso pre-release 版本，API 可能后续变动

## Task R2-8: Clippy 自动修复 + code review
- **Status**: in_progress

---

## Round 1: async-concurrency-fix (2026-07-26)

## Task 1: CdpSession 重构 (H3 + M6)
- **Status**: complete | **Commits**: ce3f7c1..b9c2f0f | **Review**: ✅ Approved

## Task 2: items 批量 + try_send (H4 + M7)
- **Status**: complete | **Commits**: b9c2f0f..a50f2f7 | **Review**: ✅ Approved

## Task 3: follow_rx Mutex 移除 (H5)
- **Status**: complete | **Commits**: a50f2f7..b1d5c88 | **Review**: ✅ Approved

## Task 4: fetch_dispatch 退避 + rule_engine 单锁 (M1 + M4)
- **Status**: complete | **Commits**: b1d5c88..1429817 | **Review**: ✅ Approved
- **Minor findings**:
  - M1: 抖动注释半开区间 vs 实现闭区间（engine.rs:453，可选修复）
  - M2: test_rule_engine_single_lock 测试有效性弱（brief 设计）
  - M3: 双 max_retries 来源（设计权衡）

## Task 5: checkpoint spawn + EventBus 并发 (M2 + M3)
- **Status**: complete | **Commits**: 1429817..3c0c3e5, b05305a | **Review**: ✅ Approved
- **Notes**:
  - 主修复 commits: 43de9d3 (EventBus 并发 + checkpoint spawn)、3c0c3e5 (ND-003-ERR 错误事件补发)
  - b05305a 修复回归：JoinSet 跟踪后台 checkpoint task，避免 delete_checkpoint 后 task 又写入；附带修复 generalize_url 误伤 v1 短版本号 + builder.rs doc test 失效（pre-existing，非本次引入）
- **Minor findings**:
  - pre-existing clippy 警告 ~600 个（clippy 1.97 新增 uninlined_format_args/unnecessary_literal_bound lint），非本次任务引入，记录至 final review 时统一处理

## Task 6: Store trait async 化 + spawn_blocking (H1 + H2)
- **Status**: complete | **Commits**: b05305a..80fa97d | **Review**: ✅ Approved
- **BASE**: b05305a | **HEAD**: 80fa97d
- **Minor findings**:
  - SqliteStore::open/open_in_memory/init_schema 仍同步（构造时一次性 PRAGMA + DDL，可后续优化）
  - 测试中 tempfile::TempDir vs tempdir() 风格小差异（不影响功能）

## Final Review + 集成验证
- **Status**: complete | **Commit**: 05e4914 (FileStore 原子写修复)
- **Final verdict**: ✅ APPROVED
- **测试基线**: 439 passed / 0 failed / 64 ignored（含 11 个新增性能测试）
- **集成验证**: banzhu-rs 编译通过
- **Final review fix**: FileStore 同 key 并发写风险（Important）→ 用 `NamedTempFile::persist` 原子替换修复
- **延后处理项**（合并后单独 PR）:
  - pre-existing clippy 警告 ~600 个（clippy 1.97 新增 uninlined_format_args/unnecessary_literal_bound lint）
  - Task 4 M1 抖动注释半开 vs 闭区间（cosmetic）
  - Task 4 M2 test_rule_engine_single_lock 测试有效性弱（设计权衡）
  - Task 6 SqliteStore::open/open_in_memory 仍同步（构造时一次性，可后续 async 化）
