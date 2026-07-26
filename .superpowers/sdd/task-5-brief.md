# Task 5: checkpoint spawn 后台 + EventBus 并发 listener（M2 + M3）

**Files:**
- Modify: `src/crawl/observability/events.rs`（EventBus::emit 改 FuturesUnordered）
- Modify: `src/crawl/runner.rs`（checkpoint 调用改 spawn 后台）
- Modify: `src/crawl/engine.rs`（persist_spider_checkpoint 签名适配，若需要）
- Test: `src/crawl/observability/events.rs`、`src/crawl/runner.rs`

**Interfaces:**
- Consumes: `futures::stream::{FuturesUnordered, StreamExt}`
- Produces: `EventBus::emit` 内部并发执行 listeners；`persist_spider_checkpoint` 调用方 spawn 后台

## Steps

### Step 1: 写失败测试 — EventBus 并发 listener

在 `src/crawl/observability/events.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_event_bus_concurrent_listeners() {
    // 验证 3 个 listener（含 1 个慢）并发执行，总延迟 ≈ max 而非 sum
    use std::time::{Duration, Instant};
    use super::*;

    let mut bus = EventBus::new();
    let counter = Arc::new(AtomicU32::new(0));
    let order = Arc::new(Mutex::new(Vec::new()));

    // 慢 listener：50ms
    let c1 = Arc::clone(&counter);
    let o1 = Arc::clone(&order);
    bus.on(move |_event: &EngineEvent| {
        let c = Arc::clone(&c1);
        let o = Arc::clone(&o1);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            c.fetch_add(1, Ordering::SeqCst);
            o.lock().await.push("slow");
        })
    });

    // 快 listener 1：5ms
    let c2 = Arc::clone(&counter);
    let o2 = Arc::clone(&order);
    bus.on(move |_event: &EngineEvent| {
        let c = Arc::clone(&c2);
        let o = Arc::clone(&o2);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            c.fetch_add(1, Ordering::SeqCst);
            o.lock().await.push("fast1");
        })
    });

    // 快 listener 2：5ms
    let c3 = Arc::clone(&counter);
    let o3 = Arc::clone(&order);
    bus.on(move |_event: &EngineEvent| {
        let c = Arc::clone(&c3);
        let o = Arc::clone(&o3);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            c.fetch_add(1, Ordering::SeqCst);
            o.lock().await.push("fast2");
        })
    });

    let start = Instant::now();
    bus.emit(EngineEvent::Started { url: "test".into() }).await;
    let elapsed = start.elapsed();

    // OPTIMIZE: 并发执行总延迟应 ≈ 50ms（最慢的），而非 60ms（串行 sum）
    assert_eq!(counter.load(Ordering::SeqCst), 3, "所有 listener 应执行");
    assert!(
        elapsed < Duration::from_millis(80),
        "并发执行应 < 80ms，实际 {:?}",
        elapsed
    );
}
```

**注意**：测试中 `bus.on(...)` 的签名需匹配现有 EventBus API。若现有 API 是 `add_listener` 或其他名称，调整测试代码。先读取 events.rs 理解现有 API。

### Step 2: 验证测试失败（旧实现串行执行）

Run: `cargo test --lib crawl::observability::events::tests::test_event_bus_concurrent_listeners --all-features`
Expected: FAIL（旧实现串行，elapsed > 80ms）

### Step 3: 实现 EventBus::emit 并发

```rust
use futures::stream::{FuturesUnordered, StreamExt};

impl EventBus {
    /// 发射事件：并发执行所有 listener，避免慢 listener 阻塞后续。
    ///
    /// OPTIMIZE: 旧实现 `for listener in &self.listeners { listener(event.clone()).await }`
    /// 串行执行，N 个 listener 中最慢的决定总延迟。
    /// 改用 FuturesUnordered 并发 await，所有 listener 同时执行，总延迟 = max(单 listener)。
    pub async fn emit(&self, event: EngineEvent) {
        if self.listeners.is_empty() {
            return;
        }
        let mut futures: FuturesUnordered<_> = self
            .listeners
            .iter()
            .map(|listener| listener(event.clone()))
            .collect();
        while futures.next().await.is_some() {}
    }
}
```

**注意**：需先读取当前 `emit` 实现，确认 listener 类型。若 listener 接收 `&EngineEvent`，需调整 clone 策略（可能需要 Arc<EngineEvent> 共享）。

### Step 4: 验证测试通过

Run: `cargo test --lib crawl::observability::events::tests::test_event_bus_concurrent_listeners --all-features`
Expected: PASS

### Step 5: 写失败测试 — checkpoint 不阻塞主循环

在 `src/crawl/runner.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_checkpoint_spawned_not_blocking_main_loop() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    let flag = Arc::new(AtomicU32::new(0));
    let flag_clone = Arc::clone(&flag);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        flag_clone.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(flag.load(Ordering::SeqCst), 0, "主循环不应等待 spawn 的任务");

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(flag.load(Ordering::SeqCst), 1, "spawn 任务应完成");
}
```

### Step 6: 验证测试通过

### Step 7: 实现 runner.rs checkpoint spawn 后台

定位 `run_inner` 函数中 checkpoint 调用段（搜索 `checkpoint` 或 `persist_spider_checkpoint`），替换为：

```rust
if pages_since_checkpoint >= self.checkpoint_interval {
    if let Some(ref store) = self.checkpoint_store {
        // OPTIMIZE: spawn 后台执行，主循环不等待；失败仅 tracing::warn。
        let store = Arc::clone(store);
        let spider_name = spider_name.clone();
        let sched = Arc::clone(&sched);
        let stats = Arc::clone(&ctx.state.stats);
        tokio::spawn(async move {
            if let Err(e) =
                engine::persist_spider_checkpoint(store.as_ref(), &spider_name, &sched, &stats)
                    .await
            {
                tracing::warn!("checkpoint 失败: {}", e);
            }
        });
    }
    pages_since_checkpoint = 0;
}
```

**注意**：需检查 `persist_spider_checkpoint` 签名。若不接受 `&Scheduler` 和 `&Arc<SpiderStats>`，需调整签名或调用方式。`spider_name` 需可 clone。

### Step 8: 验证编译

Run: `cargo build --all-features`

### Step 9: 验证测试通过

Run: `cargo test --lib crawl::observability::events::tests --all-features && cargo test --lib crawl::runner::tests --all-features`

### Step 10: 全量回归

Run: `cargo test --all-features`

### Step 11: Clippy 检查

Run: `cargo clippy --all-targets --all-features -- -D warnings`

### Step 12: 提交

```bash
cd /home/weng/wisp
git add src/crawl/observability/events.rs src/crawl/runner.rs src/crawl/engine.rs
git commit -m "perf(observability): EventBus 并发 listener + checkpoint spawn 后台"
```
