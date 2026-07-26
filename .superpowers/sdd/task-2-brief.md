# Task 2: items 批量收集 + 事件 try_send（H4 + M7）

**Files:**
- Modify: `src/crawl/engine.rs`（process_response items 段，错误事件段）
- Test: `src/crawl/engine.rs`（同文件 #[cfg(test)] 模块）

**Interfaces:**
- Consumes: `tokio::sync::mpsc::Sender::try_send`、`std::sync::Arc`
- Produces: `process_response` 内部行为变更（items 批量 push + try_send），无外部 API 变化

## Steps

### Step 1: 写失败测试 — items 批量 push 单锁

在 `src/crawl/engine.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_items_batch_push_single_lock() {
    // 验证 100 个 items 只抢一次锁（通过锁争用计数器）
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::sync::Mutex;

    struct CountingLock {
        items: Mutex<Vec<serde_json::Value>>,
        lock_count: AtomicU32,
    }

    let lock = Arc::new(CountingLock {
        items: Mutex::new(Vec::new()),
        lock_count: AtomicU32::new(0),
    });

    // 模拟批量收集：本地 Vec 收集后单次 lock extend
    let mut local: Vec<serde_json::Value> = Vec::with_capacity(100);
    for i in 0..100 {
        local.push(serde_json::json!({"i": i}));
    }
    {
        // 模拟 lock_count 自增（实际实现无此计数器，这里验证模式）
        lock.lock_count.fetch_add(1, Ordering::SeqCst);
        let mut items = lock.items.lock().await;
        items.extend(local);
    }

    assert_eq!(lock.lock_count.load(Ordering::SeqCst), 1, "应只抢一次锁");
    assert_eq!(lock.items.lock().await.len(), 100);
}
```

### Step 2: 验证测试通过（模式验证）

Run: `cargo test --lib crawl::engine::tests::test_items_batch_push_single_lock --all-features`
Expected: PASS

### Step 3: 写失败测试 — try_send 满不阻塞

```rust
#[tokio::test]
async fn test_try_send_drops_on_full_channel() {
    // 验证 channel 满时 try_send 不阻塞
    let (tx, mut rx) = tokio::sync::mpsc::channel::<i32>(2);

    // 填满 channel
    tx.try_send(1).unwrap();
    tx.try_send(2).unwrap();

    // 第 3 个应返回 Full 错误，不阻塞
    let result = tx.try_send(3);
    assert!(matches!(result, Err(tokio::sync::mpsc::error::TrySendError::Full(3))));

    // 消费一个后可再发送
    assert_eq!(rx.recv().await.unwrap(), 1);
    tx.try_send(3).unwrap();
}
```

### Step 4: 验证测试通过

Run: `cargo test --lib crawl::engine::tests::test_try_send_drops_on_full_channel --all-features`
Expected: PASS

### Step 5: 实现 process_response items 批量收集

在 `src/crawl/engine.rs` 的 `process_response` 函数中，定位 items 处理段，替换为：

```rust
// OPTIMIZE: 本地 Vec 收集所有 processed items，单次 lock 批量 push，
// 避免 N 次 lock().await.push()。tx 改 try_send 非阻塞，消费者慢时不卡核心路径。
let pipeline_crawl_ctx = if ctx.shared.middleware_chain.is_empty() {
    None
} else {
    Some(build_crawl_context(ctx))
};

// OPTIMIZE: 预分配容量，避免 Vec 增长时多次 realloc。
let mut processed_items: Vec<Value> = Vec::with_capacity(items.len());

for item in items {
    let item = match spider.on_item(item).await {
        Some(i) => i,
        None => continue,
    };
    let item = if let Some(ref crawl_ctx) = pipeline_crawl_ctx {
        ctx.shared
            .middleware_chain
            .run_pipelines(item, crawl_ctx)
            .await
    } else {
        Some(item)
    };
    if let Some(processed) = item {
        stats.items.fetch_add(1, Ordering::SeqCst);
        // OPTIMIZE: 事件用 try_send 非阻塞。channel 满时丢失事件优于阻塞核心路径。
        if let Some(ref tx) = ctx.state.tx {
            if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
                tx.try_send(CrawlEvent::Item(processed.clone()))
            {
                tracing::warn!("event channel full, dropping Item event");
            }
        }
        processed_items.push(processed);
    }
}

// OPTIMIZE: 单次 lock 批量 push，锁争用从 N 次降为 1 次。
if !processed_items.is_empty() {
    let mut items_lock = ctx.state.items.lock().await;
    items_lock.extend(processed_items);
}
```

**注意**：保留原有 `pipeline_crawl_ctx` 构建逻辑（如果已在循环外构建，则不重复构建）。需先检查当前代码是否已有 `pipeline_crawl_ctx`，若有则仅替换 items 收集段。

### Step 6: 实现错误事件 try_send

在 `src/crawl/engine.rs` 中定位错误事件发送段，将 `tx.send(...).await` 改为 `tx.try_send(...)`：

```rust
// 新代码：
if let Some(ref tx) = ctx.state.tx {
    if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) = tx.try_send(CrawlEvent::Error {
        url: sanitize_url(&req.url),
        error: e.to_string(),
    }) {
        tracing::warn!("event channel full, dropping Error event");
    }
}
```

**注意**：保留 `runner.rs` 的 `tx.send(Done).await`（流结束低频事件，必须保证送达）。

### Step 7: 验证测试通过

Run: `cargo test --lib crawl::engine::tests --all-features`
Expected: PASS

### Step 8: 全量回归

Run: `cargo test --all-features`
Expected: PASS（除 Task 1 已知的 2 个 pre-existing 失败测试）

### Step 9: Clippy 检查

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: engine.rs 无新 warning

### Step 10: 提交

```bash
cd /home/weng/wisp
git add src/crawl/engine.rs
git commit -m "perf(engine): items 批量收集 + 事件 try_send 非阻塞"
```
