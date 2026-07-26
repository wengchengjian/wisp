# Task 6 (Round 2) 报告：L5 EventListener 改 Arc<EngineEvent>

## 1. Status

DONE

## 2. Commit Hash

`feba14be0018aaeb039f6dfbab7668168bc9a699`

提交信息：`perf(events): EventListener 改 Arc<EngineEvent> 共享事件无 clone`

## 3. Test Results

### 3.1 编译

```
$ cargo build --all-features
   Compiling wisp v0.1.0 (/home/weng/wisp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.04s
```

退出码 0，编译通过。

### 3.2 全量测试

```
$ cargo test --all-features
```

统计（聚合所有 test result 行）：

| 项 | 数量 |
| --- | --- |
| passed | 435 |
| failed | 0 |
| ignored | 64 |
| measured | 0 |

全部测试 PASS，无失败。Brief 预期约 439 个，实际 435 passed + 64 ignored，符合预期范围。

### 3.3 Clippy

```
$ cargo clippy --all-targets --all-features 2>&1 | tail -5
```

退出码 0。events.rs 上的 5 个 clippy 警告全部是 `#[must_use]` 缺失建议，针对 `EventBus::new`、`has_listeners`、`listener_count`、`logging_listener`、`Metrics::new` 这些预存在 API，与本任务的 Arc 改动无关。本任务未新增 public fn 也未修改这些函数签名，因此确认无新增 clippy 警告。

其他文件的警告（`doc_lazy_continuation`、`tests/run_inner_test.rs` 的 `unused import: Store`）也都是预存在的。

## 4. Files Modified

仅 1 个文件：

- `/home/weng/wisp/src/crawl/observability/events.rs`

变更统计：1 file changed, 14 insertions(+), 13 deletions(-)

## 5. Changes Summary

按 brief 步骤执行：

1. **EventListener 类型签名**（line 102）：`Fn(EngineEvent)` → `Fn(Arc<EngineEvent>)`
2. **EventBus::emit**（line 122-138）：包裹 `Arc::new(event)`，迭代时 `Arc::clone(&event)` 共享；OPTIMIZE doc comment 同步更新，说明 Arc 共享 + FuturesUnordered 并发 await。
3. **logging_listener**（line 158-161）：闭包参数 `|event: EngineEvent|` → `|event: Arc<EngineEvent>|`，`match &event` → `match &*event`。
4. **metrics_listener**（line 188-208）：闭包参数同上，`match event`（owned）→ `match &*event`（引用），字段解引用 `*from_cache`（`&bool`）、`*elapsed_ms`（`&u64`）。
5. **测试闭包**（line 282、line 350）：`|_event: EngineEvent|` → `|_event: Arc<EngineEvent>|`。

## 6. Concerns / Notes

无。

- 仅修改 `src/crawl/observability/events.rs`，未触碰其他文件。
- 全局搜索确认 `EventListener` / `logging_listener` / `metrics_listener` 在源码中仅本文件引用，无外部 caller 受影响。
- 未添加任何向后兼容 shim，符合项目规则「从不向后兼容」。
- 无 `unwrap` 使用，符合项目规则。
- 中文注释与 OPTIMIZE 标记风格保持一致。
