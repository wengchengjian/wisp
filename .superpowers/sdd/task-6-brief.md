# Task 6 (Round 2): L5 EventListener 改 Arc<EngineEvent>

**Files:**
- Modify: `src/crawl/observability/events.rs`（仅此一个文件）

**Interfaces:**
- Consumes: `std::sync::Arc`
- Produces: `EventListener` 类型签名从 `Fn(EngineEvent)` 改为 `Fn(Arc<EngineEvent>)`；`EventBus::emit` 内部用 `Arc::new(event)` + `Arc::clone(&event)` 共享

## 背景

当前 `EventBus::emit` 实现（events.rs:127-137）：

```rust
pub async fn emit(&self, event: EngineEvent) {
    if self.listeners.is_empty() {
        return;
    }
    let mut futures: FuturesUnordered<_> = self
        .listeners
        .iter()
        .map(|listener| listener(event.clone()))  // N 次 EngineEvent clone（深拷贝）
        .collect();
    while futures.next().await.is_some() {}
}
```

每次 emit 时，N 个 listener 共享 event 需要N-1 次 `EngineEvent::clone()`（深拷贝，包含 String/Vec 字段）。
改为 `Arc<EngineEvent>` 后，仅 1 次 `Arc::new`（堆分配）+ N-1 次 `Arc::clone`（原子计数器自增，无堆分配）。

## Steps

### Step 1: 修改 EventListener 类型定义

在 `src/crawl/observability/events.rs:102`：

```rust
// 旧
pub type EventListener = Arc<dyn Fn(EngineEvent) -> BoxFuture<'static, ()> + Send + Sync>;

// 新
pub type EventListener = Arc<dyn Fn(Arc<EngineEvent>) -> BoxFuture<'static, ()> + Send + Sync>;
```

### Step 2: 修改 EventBus::emit

在 `src/crawl/observability/events.rs:127-137`：

```rust
// 旧
pub async fn emit(&self, event: EngineEvent) {
    if self.listeners.is_empty() {
        return;
    }
    let mut futures: FuturesUnordered<_> = self
        .listeners
        .iter()
        .map(|listener| listener(event.clone()))  // N 次 EngineEvent clone
        .collect();
    while futures.next().await.is_some() {}
}

// 新（Arc 共享，无 EngineEvent clone）
pub async fn emit(&self, event: EngineEvent) {
    if self.listeners.is_empty() {
        return;
    }
    let event = Arc::new(event);  // 1 次 Arc 分配
    let mut futures: FuturesUnordered<_> = self
        .listeners
        .iter()
        .map(|listener| listener(Arc::clone(&event)))  // N 次 Arc::clone（廉价）
        .collect();
    while futures.next().await.is_some() {}
}
```

同时更新 OPTIMIZE 注释，说明现在用 Arc 共享：

```rust
/// OPTIMIZE: 旧实现 `listener(event.clone())` 每次 emit N-1 次 EngineEvent 深拷贝。
/// 改用 `Arc<EngineEvent>` 共享：1 次 Arc 分配 + N-1 次 Arc::clone（原子计数器自增，无堆分配）。
/// 同时用 FuturesUnordered 并发 await，总延迟 = max(单 listener)。
```

### Step 3: 修改 logging_listener

在 `src/crawl/observability/events.rs:157-184`：

```rust
// 旧
pub fn logging_listener() -> EventListener {
    Arc::new(|event: EngineEvent| {
        Box::pin(async move {
            match &event {
                EngineEvent::CrawlStarted { spider, start_urls } => { ... }
                // ...
            }
        })
    })
}

// 新（Arc<EngineEvent> 自动 deref，match &*event 解引用）
pub fn logging_listener() -> EventListener {
    Arc::new(|event: Arc<EngineEvent>| {
        Box::pin(async move {
            match &*event {
                EngineEvent::CrawlStarted { spider, start_urls } => { ... }
                // ...
            }
        })
    })
}
```

注意：`match &event` 改为 `match &*event`（解引用 Arc 后再引用），或用 `match event.as_ref()`。

### Step 4: 修改 metrics_listener

在 `src/crawl/observability/events.rs:187-223`：

```rust
// 旧
pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener {
    Arc::new(move |event: EngineEvent| {
        let metrics = Arc::clone(&metrics);
        Box::pin(async move {
            match event {  // event 是 owned EngineEvent
                EngineEvent::ResponseReceived { elapsed_ms, from_cache, .. } => {
                    metrics.responses.fetch_add(1, ...);
                    if from_cache {  // from_cache 是 bool
                        metrics.cache_hits.fetch_add(1, ...);
                    }
                    metrics.total_elapsed_ms.fetch_add(elapsed_ms, ...);  // elapsed_ms 是 u64
                }
                EngineEvent::ItemScraped { .. } => { ... }
                EngineEvent::ErrorOccurred { .. } => { ... }
                _ => {}
            }
        })
    })
}

// 新（event 是 Arc<EngineEvent>，match &*event 改为引用解构）
pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener {
    Arc::new(move |event: Arc<EngineEvent>| {
        let metrics = Arc::clone(&metrics);
        Box::pin(async move {
            match &*event {  // 解引用 Arc，match 引用
                EngineEvent::ResponseReceived { elapsed_ms, from_cache, .. } => {
                    metrics.responses.fetch_add(1, ...);
                    if *from_cache {  // from_cache 是 &bool，需要解引用
                        metrics.cache_hits.fetch_add(1, ...);
                    }
                    metrics.total_elapsed_ms.fetch_add(*elapsed_ms, ...);  // elapsed_ms 是 &u64
                }
                EngineEvent::ItemScraped { .. } => { ... }
                EngineEvent::ErrorOccurred { .. } => { ... }
                _ => {}
            }
        })
    })
}
```

注意：`match event`（owned）改为 `match &*event`（引用），字段 `elapsed_ms` 从 `u64` 变为 `&u64`，需要 `*elapsed_ms` 解引用。`from_cache` 从 `bool` 变为 `&bool`，需要 `*from_cache` 解引用（或在 if 条件中自动 deref）。

### Step 5: 修改测试中的 listener 闭包

在 `src/crawl/observability/events.rs:281` 和 `:349`：

```rust
// 旧（line 281）
bus.on(Arc::new(move |_event: EngineEvent| {
    let c = Arc::clone(&counter_clone);
    Box::pin(async move {
        c.fetch_add(1, Ordering::SeqCst);
    })
}));

// 新
bus.on(Arc::new(move |_event: Arc<EngineEvent>| {
    let c = Arc::clone(&counter_clone);
    Box::pin(async move {
        c.fetch_add(1, Ordering::SeqCst);
    })
}));
```

```rust
// 旧（line 349）
bus.on(Arc::new(move |_event: EngineEvent| {
    let c = Arc::clone(&c);
    let o = Arc::clone(&o);
    Box::pin(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.fetch_add(1, Ordering::SeqCst);
        o.lock().await.push(label);
    })
}));

// 新
bus.on(Arc::new(move |_event: Arc<EngineEvent>| {
    let c = Arc::clone(&c);
    let o = Arc::clone(&o);
    Box::pin(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.fetch_add(1, Ordering::SeqCst);
        o.lock().await.push(label);
    })
}));
```

### Step 6: 编译验证

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

### Step 7: 全量回归

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS（约 439 个）

### Step 8: Clippy 检查

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features 2>&1 | tail -5`
Expected: 不增加新警告（已有的警告保持不变）

### Step 9: 提交

```bash
cd /home/weng/wisp
git add src/crawl/observability/events.rs
git commit -m "perf(events): EventListener 改 Arc<EngineEvent> 共享事件无 clone"
```

## 验证

- 编译通过
- 所有测试 PASS
- 无新增 clippy 警告
- 提交信息符合规范：`perf(events): EventListener 改 Arc<EngineEvent> 共享事件无 clone`
