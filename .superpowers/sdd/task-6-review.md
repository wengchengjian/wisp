# Task 6 Review

## Spec compliance

1. ✅ `EventListener` 类型已改为 `Arc<dyn Fn(Arc<EngineEvent>) -> BoxFuture<'static, ()> + Send + Sync>`（events.rs:102）。
2. ✅ `EventBus::emit` 在 line 131 `let event = Arc::new(event);`，line 135 `listener(Arc::clone(&event))`，无 `event.clone()`，符合「1 次 Arc 分配 + N-1 次 Arc::clone」要求。
3. ✅ `logging_listener` 闭包签名改为 `|event: Arc<EngineEvent>|`（line 159），`match &*event`（line 161）。原 `match &event`（`&EngineEvent`）与新 `match &*event`（同样 `&EngineEvent`）匹配语义等价。
4. ✅ `metrics_listener` 闭包签名改为 `|event: Arc<EngineEvent>|`（line 189），`match &*event`（line 192），字段 `*from_cache`（line 201，`&bool` → `bool`）、`*elapsed_ms`（line 208，`&u64` → `u64`）均正确解引用。
5. ✅ 测试闭包签名更新：line 282（`test_event_bus_with_listener`）、line 350（`test_event_bus_concurrent_listeners`）均改为 `|_event: Arc<EngineEvent>|`。
6. ✅ 仅 `src/crawl/observability/events.rs` 被修改（diff 统计 `1 file changed, 14 insertions(+), 13 deletions(-)`）。全局 Grep 确认 `EventListener`/`logging_listener`/`metrics_listener` 在源码中仅本文件引用，无外部 caller 受影响。

## Code quality

7. ✅ 无向后兼容 shim，无多余抽象。直接修改类型签名，符合「从不向后兼容」。
8. ✅ 无 `unwrap()` 引入，本任务也不需要 `expect`。
9. ✅ 中文注释保持，OPTIMIZE 标记已更新（line 124-126），合并说明 Arc 共享 + FuturesUnordered 并发 await，准确反映当前实现。Task 5 的并发说明保留（避免丢失历史上下文）。
10. ✅ `Arc` 已在原文件 line 19 导入（`use std::sync::Arc;`），diff 未添加重复 import。

## Test quality

11. ✅ 测试逻辑保持：`test_event_bus_with_listener` 仍验证计数器递增到 2；`test_event_bus_concurrent_listeners` 仍验证计数器=3 + 并发延迟 < 80ms；`test_metrics_listener` 仍验证 responses/items/avg_response_ms。断言未被弱化。
12. ✅ 无测试被删除或弱化，仅闭包签名跟随类型变更更新。
13. ⚠️ 报告声称 435 passed / 0 failed / 64 ignored，符合 brief 预期「约 439 个」范围（435 与 439 差 4，brief 用「约」表述，无 failed）。无法从 diff 直接验证，但报告数据自洽。

## Verdict
APPROVED

## Findings (if any)
无。

## 备注
- 实现完全符合 brief 步骤 1-5，无遗漏、无多余改动。
- `match &*event` 选择优于 `event.as_ref()`，与 brief 示例一致，风格统一。
- OPTIMIZE 注释更新合理：既保留 Task 5 的并发说明（仍准确），又补充 Task 6 的 Arc 共享说明，无信息丢失。
- 提交信息 `perf(events): EventListener 改 Arc<EngineEvent> 共享事件无 clone` 符合项目规范（中文、perf 类型）。
