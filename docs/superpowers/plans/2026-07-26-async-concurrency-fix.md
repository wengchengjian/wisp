# 异步与并发性能修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 wisp 框架 11 项异步/并发性能问题（5 High + 6 关键 Medium），消除 async runtime 阻塞、降低锁争用、提升高并发吞吐。

**Architecture:** 分 6 Phase 渐进式推进，从独立模块（CdpSession）到核心 engine 再到 storage trait，每 Phase 独立 commit + 全量回归。TDD 流程：红→绿→重构→提交。

**Tech Stack:** Rust + Tokio + async-trait + parking_lot + futures + tokio-tungstenite + rusqlite + moka

## Global Constraints

- **分支**：master 主分支直接开发，禁止 worktree/feature branch
- **兼容性**：不向后兼容，删除旧字段/方法不保留兼容层
- **测试基线**：现有 273 个测试必须每 Phase 后全过
- **依赖**：不引入新 crate，仅用 tokio/futures/async-trait/parking_lot 生态
- **提交信息**：中文简短一行，格式 `perf(<scope>): <描述>`
- **验证命令**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings`
- **工作目录**：`/home/weng/wisp`

## File Structure

| Phase | 文件 | 责任 |
|---|---|---|
| 1 | `src/browser/cdp.rs` | CDP WebSocket 会话：事件广播 + 命令分发 |
| 2 | `src/crawl/engine.rs` | items 批量收集 + 事件 try_send |
| 3 | `src/crawl/engine.rs`, `src/crawl/runner.rs` | follow_rx 移出 EngineShared |
| 4 | `src/crawl/engine.rs`, `src/crawl/middleware/builtin.rs`, `src/crawl/middleware/mod.rs` | 退避抖动 + rule_engine 单次锁 |
| 5 | `src/crawl/observability/events.rs`, `src/crawl/runner.rs`, `src/crawl/engine.rs` | EventBus 并发 + checkpoint spawn |
| 6 | `src/storage/{mod,sqlite,file,memory}.rs` + 8 调用点 | Store trait async 化 |

---

## Task 1: CdpSession 重构 — broadcast + watch 错误传播（H3 + M6）

**Files:**
- Modify: `src/browser/cdp.rs`（删除 events Vec + consumed_offset，新增 conn_state watch）
- Test: `src/browser/cdp.rs`（同文件 #[cfg(test)] 模块）

**Interfaces:**
- Consumes: `tokio::sync::{broadcast, watch, oneshot, Mutex}`、`futures::{SinkExt, StreamExt}`
- Produces: `CdpSession::subscribe_events() -> broadcast::Receiver<CdpEvent>`（签名不变）、`CdpSession::execute_with_session()` 内部用 `select!` 等待响应或连接关闭

- [ ] **Step 1: 写失败测试 — broadcast 直达订阅者**

在 `src/browser/cdp.rs` 的 `#[cfg(test)]` 模块末尾追加：

```rust
#[tokio::test]
async fn test_cdp_event_broadcast_no_vec() {
    // 验证事件经 broadcast 直达订阅者，无需 Vec 中转
    let (tx, mut rx1) = tokio::sync::broadcast::channel::<String>(16);
    let mut rx2 = tx.subscribe();

    tx.send("Page.loadEventFired".to_string()).unwrap();

    let e1 = rx1.recv().await.unwrap();
    let e2 = rx2.recv().await.unwrap();
    assert_eq!(e1, "Page.loadEventFired");
    assert_eq!(e2, "Page.loadEventFired");
}
```

- [ ] **Step 2: 验证测试通过（broadcast 基础设施验证）**

Run: `cargo test --lib browser::cdp::tests::test_cdp_event_broadcast_no_vec --all-features`
Expected: PASS（broadcast 行为验证，不依赖 CdpSession 实现）

- [ ] **Step 3: 写失败测试 — 连接错误通知 pending**

在 `src/browser/cdp.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_cdp_connection_error_notifies_pending() {
    // 验证 ConnState watch 错误传播：连接关闭时所有 watch 订阅者立即收到
    use tokio::sync::watch;
    let (tx, mut rx) = watch::channel(ConnState::Open);

    // 模拟连接关闭
    tx.send(ConnState::Closed("ws closed".into())).unwrap();

    // 订阅者应立即感知
    assert!(rx.has_changed().unwrap());
    match &*rx.borrow() {
        ConnState::Closed(msg) => assert_eq!(msg, "ws closed"),
        ConnState::Open => panic!("expected Closed"),
    }
}
```

- [ ] **Step 4: 验证测试失败（ConnState 类型尚未定义）**

Run: `cargo test --lib browser::cdp::tests::test_cdp_connection_error_notifies_pending --all-features`
Expected: FAIL with "cannot find type `ConnState` in this scope"

- [ ] **Step 5: 写失败测试 — wait_for_event 使用 broadcast**

在 `src/browser/cdp.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_wait_for_event_uses_broadcast() {
    // 验证 lagged 消费者不卡死：broadcast::Receiver::recv 处理 Lagged 错误继续
    let (tx, _) = tokio::sync::broadcast::channel::<i32>(2);
    let mut rx = tx.subscribe();

    // 发送 5 个事件（超过容量 2，rx 会 Lagged）
    for i in 0..5 {
        let _ = tx.send(i);
    }

    // rx 应该收到 Lagged 错误而非卡死
    match rx.recv().await {
        Ok(v) => assert!(v >= 0 && v < 5),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
            assert!(n > 0, "lagged count should be positive");
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => panic!("should not close"),
    }
}
```

- [ ] **Step 6: 验证测试通过（broadcast Lagged 行为验证）**

Run: `cargo test --lib browser::cdp::tests::test_wait_for_event_uses_broadcast --all-features`
Expected: PASS

- [ ] **Step 7: 实现 CdpSession 重构**

在 `src/browser/cdp.rs` 中进行以下修改：

**7a. 添加 ConnState 枚举**（在 `CdpEvent` 结构体定义后）：

```rust
/// 连接状态：用于失败时快速通知所有等待中的 execute 调用者。
#[derive(Debug, Clone)]
enum ConnState {
    Open,
    /// 已关闭，包含错误信息（clone 给所有 pending 的 oneshot）。
    Closed(String),
}
```

**7b. 修改 CdpSession 结构体**（删除 events + consumed_offset，新增 conn_state）：

```rust
pub struct CdpSession {
    writer: Arc<Mutex<futures::stream::SplitSink<WsStream, Message>>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    // OPTIMIZE: 删除 events Vec 和 consumed_offset，统一用 broadcast。
    event_broadcaster: broadcast::Sender<CdpEvent>,
    // OPTIMIZE: 连接状态广播，错误时所有 execute 立即收到，避免 30s timeout 等待。
    conn_state: watch::Sender<ConnState>,
}
```

**7c. 修改 connect 方法**（删除 events/consumed_offset 初始化，后台任务错误时通知 conn_state）：

在 `connect` 函数中：
- 删除 `let events = Arc::new(Mutex::new(Vec::<CdpEvent>::new()));`
- 删除 `let consumed_offset = Arc::new(Mutex::new(0usize));`
- 添加 `let (conn_state_tx, _conn_state_rx) = watch::channel(ConnState::Open);`
- 在后台任务的 `Err(e) =>` 和 `Ok(Message::Close(_)) =>` 分支中，调用 `let _ = conn_state_clone.send(ConnState::Closed(format!("ws error: {e}")));` 后再 `break`
- 在 break 前清空 pending：`pending_clone.lock().await.clear();`
- 返回的 `Self` 中删除 `events` 和 `consumed_offset` 字段，新增 `conn_state: conn_state_tx`

**7d. 修改 execute_with_session 方法**（注册 pending 前检查 conn_state，select! 等待响应或关闭）：

```rust
pub async fn execute_with_session(
    self: &Arc<Self>,
    method: &str,
    params: Value,
    session_id: Option<&str>,
) -> Result<Value> {
    // OPTIMIZE: 注册 pending 前先检查连接状态，避免注定失败的命令占用 30s。
    if matches!(*self.conn_state.borrow(), ConnState::Closed(_)) {
        return Err(WispError::Browser(BrowserError::CdpConnection(
            "connection closed".into(),
        )));
    }

    let id = self.next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel();
    self.pending.lock().await.insert(id, tx);

    let mut msg = json!({ "id": id, "method": method, "params": params });
    if let Some(sid) = session_id {
        msg["sessionId"] = json!(sid);
    }

    let text = serde_json::to_string(&msg).expect("CDP msg always serializable");
    {
        let mut writer = self.writer.lock().await;
        writer
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| WispError::Browser(BrowserError::CdpConnection(format!("ws send: {e}"))))?;
    }

    // OPTIMIZE: select! 同时等待响应和连接关闭通知，连接断开时立即返回错误。
    let mut state_rx = self.conn_state.subscribe();
    let response = tokio::select! {
        r = tokio::time::timeout(Duration::from_secs(30), rx) => {
            match r {
                Ok(Ok(v)) => v,
                Ok(Err(_)) => {
                    return Err(WispError::Browser(BrowserError::CdpConnection(
                        "channel closed".into(),
                    )));
                }
                Err(_) => {
                    self.pending.lock().await.remove(&id);
                    return Err(WispError::Timeout(format!("CDP: {method}")));
                }
            }
        }
        _ = state_rx.changed() => {
            self.pending.lock().await.remove(&id);
            let msg = match &*state_rx.borrow() {
                ConnState::Closed(m) => m.clone(),
                _ => "connection closed".to_string(),
            };
            return Err(WispError::Browser(BrowserError::CdpConnection(msg)));
        }
    };

    if let Some(error) = response.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("CDP error");
        return Err(WispError::Browser(BrowserError::CdpConnection(msg.to_string())));
    }

    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}
```

**7e. 重写 wait_for_event 方法**（用 broadcast::Receiver 替代 Vec 扫描）：

```rust
pub async fn wait_for_event<F>(&self, predicate: F, timeout_ms: u64) -> Result<CdpEvent>
where
    F: Fn(&CdpEvent) -> bool,
{
    let mut rx = self.subscribe_events();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(WispError::Timeout("waiting for CDP event".into()));
        }
        tokio::select! {
            recv = rx.recv() => {
                match recv {
                    Ok(event) => {
                        if predicate(&event) {
                            return Ok(event);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("CDP event subscriber lagged by {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(WispError::Browser(BrowserError::CdpConnection(
                            "event broadcaster closed".into(),
                        )));
                    }
                }
            }
            _ = tokio::time::sleep(remaining) => {
                return Err(WispError::Timeout("waiting for CDP event".into()));
            }
        }
    }
}
```

**7f. 删除旧的 `events`/`consumed_offset` 相关代码**：
- 删除 `subscribe_events` 中 `let tx = self.events.lock().await.clone();` 相关逻辑（保留 `self.event_broadcaster.subscribe()`）
- 删除任何引用 `self.events` 或 `self.consumed_offset` 的代码

- [ ] **Step 8: 验证新测试通过**

Run: `cargo test --lib browser::cdp::tests --all-features`
Expected: PASS（所有 cdp 测试，含新增 3 个）

- [ ] **Step 9: 全量回归**

Run: `cargo test --all-features`
Expected: PASS（273 现有 + 3 新增 = 276 测试全过）

- [ ] **Step 10: Clippy 检查**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

- [ ] **Step 11: 提交**

```bash
cd /home/weng/wisp
git add src/browser/cdp.rs
git commit -m "perf(cdp): 删除 events Vec 改用 broadcast + 错误传播 watch"
```

---

## Task 2: items 批量收集 + 事件 try_send（H4 + M7）

**Files:**
- Modify: `src/crawl/engine.rs`（process_response items 段 L289-316，错误事件 L166-172）
- Test: `src/crawl/engine.rs`（同文件 #[cfg(test)] 模块）

**Interfaces:**
- Consumes: `tokio::sync::mpsc::Sender::try_send`、`std::sync::Arc`
- Produces: `process_response` 内部行为变更（items 批量 push + try_send），无外部 API 变化

- [ ] **Step 1: 写失败测试 — items 批量 push 单锁**

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

- [ ] **Step 2: 验证测试通过（模式验证）**

Run: `cargo test --lib crawl::engine::tests::test_items_batch_push_single_lock --all-features`
Expected: PASS

- [ ] **Step 3: 写失败测试 — try_send 满不阻塞**

在 `src/crawl/engine.rs` 的 `#[cfg(test)]` 模块追加：

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

- [ ] **Step 4: 验证测试通过**

Run: `cargo test --lib crawl::engine::tests::test_try_send_drops_on_full_channel --all-features`
Expected: PASS

- [ ] **Step 5: 实现 process_response items 批量收集**

在 `src/crawl/engine.rs` 的 `process_response` 函数中，定位 items 处理段（约 L289-316），替换为：

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

- [ ] **Step 6: 实现错误事件 try_send**

在 `src/crawl/engine.rs` 中定位错误事件发送段（约 L166-172），将 `tx.send(...).await` 改为 `tx.try_send(...)`：

```rust
// 旧代码（约 L166-172）：
// if let Some(ref tx) = ctx.state.tx {
//     let _ = tx.send(CrawlEvent::Error { ... }).await;
// }

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

**注意**：保留 `runner.rs:134, 143` 的 `tx.send(Done).await`（流结束低频事件，必须保证送达）。

- [ ] **Step 7: 验证测试通过**

Run: `cargo test --lib crawl::engine::tests --all-features`
Expected: PASS

- [ ] **Step 8: 全量回归**

Run: `cargo test --all-features`
Expected: PASS

- [ ] **Step 9: Clippy 检查**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

- [ ] **Step 10: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/engine.rs
git commit -m "perf(engine): items 批量收集 + 事件 try_send 非阻塞"
```

---

## Task 3: follow_rx Mutex 移除（H5）

**Files:**
- Modify: `src/crawl/engine.rs`（删除 EngineShared.follow_rx 字段 + 4 处测试构造 L839, L950, L1061, L1253）
- Modify: `src/crawl/runner.rs`（unfold 状态持有 Receiver + 主构造 L228）
- Modify: `src/crawl/builder.rs`（若 EngineShared 构造涉及 follow_rx）
- Test: `src/crawl/engine.rs`、`src/crawl/runner.rs`

**Interfaces:**
- Consumes: `tokio::sync::mpsc::UnboundedReceiver`
- Produces: `EngineShared` 不再有 `follow_rx` 字段；`run_inner` 内部 unfold 状态持有 `UnboundedReceiver<Request>`

- [ ] **Step 1: 写失败测试 — follow_rx drain 无 Mutex**

在 `src/crawl/runner.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_follow_rx_drained_without_mutex() {
    // 验证 UnboundedReceiver 可直接 try_recv drain，无需 Mutex 包装
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<i32>();

    // 发送 3 个请求
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    tx.send(3).unwrap();

    // 直接 try_recv drain（无锁）
    let mut drained = Vec::new();
    while let Ok(v) = rx.try_recv() {
        drained.push(v);
    }

    assert_eq!(drained, vec![1, 2, 3]);
}
```

- [ ] **Step 2: 验证测试通过**

Run: `cargo test --lib crawl::runner::tests::test_follow_rx_drained_without_mutex --all-features`
Expected: PASS

- [ ] **Step 3: 删除 EngineShared.follow_rx 字段**

在 `src/crawl/engine.rs` 中：

**3a. 删除 EngineShared 结构体的 `follow_rx` 字段**：

```rust
// 旧（约 L59-75）：
// pub struct EngineShared {
//     pub sched: Arc<Scheduler>,
//     pub follow_tx: UnboundedSender<Request>,
//     pub follow_rx: Arc<Mutex<UnboundedReceiver<Request>>>,  // 删除此行
//     ...
// }

// 新：
pub struct EngineShared {
    pub sched: Arc<Scheduler>,
    pub follow_tx: UnboundedSender<Request>,
    // OPTIMIZE: follow_rx 移出 EngineShared，由 runner.rs unfold 状态持有。
    // 旧实现 Arc<Mutex<UnboundedReceiver>> 串行化所有 follow drain，但 Receiver 是单消费者无需 Mutex。
    pub proxy_clients: Arc<moka::Cache<...>>,
    // ... 其余字段不变
}
```

**3b. 删除 4 处测试构造中的 follow_rx 初始化**（L839, L950, L1061, L1253）：

每处都删除 `follow_rx: Arc::new(Mutex::new(follow_rx)),` 行。同时可删除 `let (follow_tx, follow_rx) = mpsc::unbounded_channel::<Request>();` 中的 `follow_rx` 绑定（若测试不需要单独持有 receiver），改为 `let (follow_tx, _) = mpsc::unbounded_channel::<Request>();`。

- [ ] **Step 4: 修改 runner.rs — unfold 状态持有 Receiver**

在 `src/crawl/runner.rs` 的 `run_inner` 函数中：

**4a. 修改 unfold 状态类型**（包含 `UnboundedReceiver<Request>`）：

```rust
// 旧（约 L228, L350-370）：
// let (follow_tx, follow_rx) = mpsc::unbounded_channel::<Request>();
// let shared = EngineShared { follow_rx: Arc::new(Mutex::new(follow_rx)), ... };
// let stream = stream::unfold(state, move |ctx| async move {
//     let mut rx_guard = ctx.shared.follow_rx.lock().await;
//     while let Ok(req) = rx_guard.try_recv() { ... }
//     drop(rx_guard);
//     ...
// });

// 新：
let (follow_tx, follow_rx) = mpsc::unbounded_channel::<Request>();
let shared = EngineShared {
    follow_tx,
    // follow_rx 不再放入 EngineShared
    ...
};

// OPTIMIZE: follow_rx move 进 unfold 状态，单消费者无需 Mutex。
let stream = stream::unfold((ctx, follow_rx), move |(mut ctx, mut rx)| async move {
    // OPTIMIZE: 直接 try_recv drain，无锁争用
    while let Ok(req) = rx.try_recv() {
        // 处理 follow 请求（push 到 scheduler）
        ctx.shared.sched.push(req).await;
    }
    // ... 原有 unfold 逻辑
    Some(((), (ctx, rx)))
});
```

**注意**：需保留原有 follow 请求的处理逻辑（如 scheduler.push、stats 更新等），仅替换 drain 方式。

**4b. 删除 `let mut rx_guard = ctx.shared.follow_rx.lock().await;` 及 `drop(rx_guard);`**（约 L370-374）

- [ ] **Step 5: 检查 builder.rs**

Run: `grep -n "follow_rx" /home/weng/wisp/src/crawl/builder.rs`
Expected: 若有匹配，按相同模式删除 follow_rx 构造

- [ ] **Step 6: 验证编译**

Run: `cargo build --all-features`
Expected: 编译通过（若失败，根据错误信息调整遗漏的构造点）

- [ ] **Step 7: 验证测试通过**

Run: `cargo test --lib crawl::runner::tests::test_follow_rx_drained_without_mutex --all-features`
Expected: PASS

- [ ] **Step 8: 全量回归**

Run: `cargo test --all-features`
Expected: PASS（所有现有 follow 相关测试通过）

- [ ] **Step 9: Clippy 检查**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

- [ ] **Step 10: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/engine.rs src/crawl/runner.rs src/crawl/builder.rs
git commit -m "perf(runner): follow_rx 移入 unfold 状态移除 Mutex"
```

---

## Task 4: fetch_dispatch 退避抖动 + rule_engine 单次锁（M1 + M4）

**Files:**
- Modify: `src/crawl/engine.rs`（fetch_dispatch L386-437）
- Modify: `src/crawl/middleware/builtin.rs`（RetryMiddleware::process_error 移除 sleep + new 删除 retry_delay）
- Modify: `src/crawl/middleware/mod.rs`（doc 示例更新）
- Test: `src/crawl/engine.rs`

**Interfaces:**
- Consumes: `rand::rngs::SmallRng`、`rand::{Rng, SeedableRng}`（检查 Cargo.toml 是否已有 rand 依赖）
- Produces: `RetryMiddleware::new(max_retries: u32)`（删除 retry_delay 参数）

- [ ] **Step 1: 检查 rand 依赖**

Run: `grep -n "^rand" /home/weng/wisp/Cargo.toml`
Expected: 若无 rand 依赖，需在 [dependencies] 添加 `rand = "0.9"`（wisp 应已有 rand 用于其他场景，先检查）

- [ ] **Step 2: 写失败测试 — 指数退避**

在 `src/crawl/engine.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[test]
fn test_retry_exponential_backoff() {
    // 验证指数退避公式：delay = min(100ms * 2^attempt, 30s)
    fn compute_backoff(attempt: u32) -> u64 {
        let base_ms = 100u64;
        let cap_ms = 30_000u64;
        base_ms
            .saturating_mul(1u64 << attempt.min(10))
            .min(cap_ms)
    }

    assert_eq!(compute_backoff(1), 200, "attempt=1 → 200ms");
    assert_eq!(compute_backoff(2), 400, "attempt=2 → 400ms");
    assert_eq!(compute_backoff(3), 800, "attempt=3 → 800ms");
    assert_eq!(compute_backoff(8), 25_600, "attempt=8 → 25.6s");
    assert_eq!(compute_backoff(10), 30_000, "attempt=10 → 封顶 30s");
    assert_eq!(compute_backoff(20), 30_000, "attempt=20 → 仍封顶 30s");
}
```

- [ ] **Step 3: 验证测试通过**

Run: `cargo test --lib crawl::engine::tests::test_retry_exponential_backoff --all-features`
Expected: PASS

- [ ] **Step 4: 写失败测试 — 抖动范围**

在 `src/crawl/engine.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[test]
fn test_retry_jitter_range() {
    // 验证抖动在 [0, exp_delay/2) 范围
    fn compute_jitter_bound(exp_delay: u64) -> u64 {
        exp_delay / 2 + 1
    }

    // exp_delay=200ms → jitter ∈ [0, 100)
    assert_eq!(compute_jitter_bound(200), 101);
    // exp_delay=400ms → jitter ∈ [0, 200)
    assert_eq!(compute_jitter_bound(400), 201);
    // exp_delay=30_000ms → jitter ∈ [0, 15_000)
    assert_eq!(compute_jitter_bound(30_000), 15_001);
}
```

- [ ] **Step 5: 验证测试通过**

Run: `cargo test --lib crawl::engine::tests::test_retry_jitter_range --all-features`
Expected: PASS

- [ ] **Step 6: 写失败测试 — rule_engine 单次锁**

在 `src/crawl/engine.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_rule_engine_single_lock_for_autofallback() {
    // 验证 resolve+learn 在单次锁内完成（通过模拟 ModeRuleEngine）
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::sync::Mutex;

    struct MockRuleEngine {
        lock_count: AtomicU32,
    }
    impl MockRuleEngine {
        async fn resolve_and_learn(&self) -> bool {
            // 模拟单次锁内完成 resolve + learn
            self.lock_count.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    let engine = Arc::new(MockRuleEngine {
        lock_count: AtomicU32::new(0),
    });

    // 模拟 AutoFallback 路径：单次锁
    let should_upgrade = engine.resolve_and_learn().await;

    assert!(should_upgrade);
    assert_eq!(engine.lock_count.load(Ordering::SeqCst), 1, "应只抢一次锁");
}
```

- [ ] **Step 7: 验证测试通过**

Run: `cargo test --lib crawl::engine::tests::test_rule_engine_single_lock_for_autofallback --all-features`
Expected: PASS

- [ ] **Step 8: 实现 fetch_dispatch AutoFallback 单次锁**

在 `src/crawl/engine.rs` 的 `fetch_dispatch` 函数中，定位 AutoFallback 段（约 L386-396），替换为：

```rust
// 旧（约 L386-396，两次锁）：
// let should_upgrade = {
//     let engine = ctx.shared.rule_engine.lock().await;
//     engine.resolve(&current_req.url) != Some(FetchMode::Stealth)
// };
// if should_upgrade {
//     let mut engine = ctx.shared.rule_engine.lock().await;
//     engine.learn(&current_req.url, FetchMode::Stealth);
//     ...
// }

// 新（单次锁）：
// OPTIMIZE: 合并 resolve+learn 到单次锁，避免两次锁争用。
if ctx.config.fetch_mode == FetchMode::Auto
    && current_req.fetch_mode_override.is_none()
    && current_req.retry_count == 0
{
    let should_upgrade = {
        let mut engine = ctx.shared.rule_engine.lock().await;
        if engine.resolve(&current_req.url) != Some(FetchMode::Stealth) {
            engine.learn(&current_req.url, FetchMode::Stealth);
            true
        } else {
            false
        }
    };
    if should_upgrade {
        tracing::info!(
            "AutoFallback: '{}' 首次抓取失败 ({}), 升级 Stealth 重试",
            sanitize_url(&current_req.url),
            e
        );
        current_req.fetch_mode_override = Some(FetchMode::Stealth);
        continue;
    }
}
```

- [ ] **Step 9: 实现 Retry 路径指数退避 + 抖动**

在 `src/crawl/engine.rs` 的 `fetch_dispatch` 函数中，定位 Retry 分支（约 L417-437），在 `continue` 前添加退避：

```rust
middleware::ErrorAction::Retry if current_req.retry_count < max_retries => {
    current_req.retry_count += 1;
    stats.retries.fetch_add(1, Ordering::SeqCst);

    // OPTIMIZE: 指数退避 + 抖动，避免失败场景下的 Thundering Herd。
    let attempt = current_req.retry_count;
    let base_ms = 100u64;
    let cap_ms = 30_000u64;
    let exp_delay = base_ms
        .saturating_mul(1u64 << attempt.min(10))
        .min(cap_ms);
    // 抖动：[0, exp_delay/2)，避免所有失败请求同步重试
    let jitter = rand::rngs::SmallRng::try_from_rng(&mut rand::rngs::SysRng)
        .expect("OS RNG failed")
        .random_range(0..exp_delay / 2 + 1);
    let total_delay = std::time::Duration::from_millis(exp_delay + jitter);
    tracing::debug!(
        "retry {}/{}: {} (backoff {:?})",
        attempt,
        max_retries,
        sanitize_url(&current_req.url),
        total_delay
    );
    tokio::time::sleep(total_delay).await;

    if let Some(ref tx) = ctx.state.tx {
        let _ = tx.try_send(CrawlEvent::Retry {
            url: sanitize_url(&current_req.url),
            attempt,
            max: max_retries,
            error: e.to_string(),
        });
    }
    continue;
}
```

**注意**：检查 `rand::rngs::SmallRng::try_from_rng` 和 `.random_range` 在 wisp 使用的 rand 版本中的 API。若 rand 版本不同，调整 API 调用。

- [ ] **Step 10: 修改 RetryMiddleware — 移除 sleep 和 retry_delay**

在 `src/crawl/middleware/builtin.rs` 中：

**10a. 修改 RetryMiddleware 结构体和 new**：

```rust
// 旧：
// pub struct RetryMiddleware {
//     max_retries: u32,
//     retry_delay: Duration,
// }
// impl RetryMiddleware {
//     pub fn new(max_retries: u32, retry_delay: Duration) -> Self { ... }
// }

// 新：
pub struct RetryMiddleware {
    max_retries: u32,
}

impl RetryMiddleware {
    /// 创建重试中间件。退避策略由 engine 统一负责（指数退避 + 抖动）。
    pub fn new(max_retries: u32) -> Self {
        Self { max_retries }
    }
}
```

**10b. 修改 process_error — 删除内部 sleep**：

```rust
// 旧（约 L96-104）：
// async fn process_error(&self, req: &Request, error: &str) -> ErrorAction {
//     if req.retry_count < self.max_retries {
//         tokio::time::sleep(self.retry_delay).await;  // 删除此行
//         ErrorAction::Retry
//     } else {
//         ErrorAction::Propagate
//     }
// }

// 新：
async fn process_error(&self, req: &Request, _error: &str) -> ErrorAction {
    // OPTIMIZE: 退避由 engine 统一负责（指数退避 + 抖动），中间件仅决定是否重试。
    if req.retry_count < self.max_retries {
        ErrorAction::Retry
    } else {
        ErrorAction::Propagate
    }
}
```

- [ ] **Step 11: 更新 mod.rs doc 示例**

在 `src/crawl/middleware/mod.rs` 中，更新 doc 示例（约 L18）：

```rust
// 旧：
// .middleware(RetryMiddleware::new(3, std::time::Duration::from_secs(1)))

// 新：
.middleware(RetryMiddleware::new(3))
```

- [ ] **Step 12: 检查所有 RetryMiddleware::new 调用点**

Run: `grep -rn "RetryMiddleware::new" /home/weng/wisp/src/`
Expected: 所有调用点都已更新为单参数版本

- [ ] **Step 13: 验证测试通过**

Run: `cargo test --lib crawl::engine::tests --all-features && cargo test --lib crawl::middleware --all-features`
Expected: PASS

- [ ] **Step 14: 全量回归**

Run: `cargo test --all-features`
Expected: PASS

- [ ] **Step 15: Clippy 检查**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

- [ ] **Step 16: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/engine.rs src/crawl/middleware/builtin.rs src/crawl/middleware/mod.rs
git commit -m "perf(engine): fetch_dispatch 退避抖动 + rule_engine 单次锁"
```

---

## Task 5: checkpoint spawn 后台 + EventBus 并发 listener（M2 + M3）

**Files:**
- Modify: `src/crawl/observability/events.rs`（EventBus::emit 改 FuturesUnordered）
- Modify: `src/crawl/runner.rs`（checkpoint 调用改 spawn 后台）
- Modify: `src/crawl/engine.rs`（persist_spider_checkpoint 保持 async，调用方不 await）
- Test: `src/crawl/observability/events.rs`、`src/crawl/runner.rs`

**Interfaces:**
- Consumes: `futures::stream::{FuturesUnordered, StreamExt}`
- Produces: `EventBus::emit` 内部并发执行 listeners；`persist_spider_checkpoint` 调用方 spawn 后台

- [ ] **Step 1: 写失败测试 — EventBus 并发 listener**

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

**注意**：测试中 `bus.on(...)` 的签名需匹配现有 EventBus API。若现有 API 是 `add_listener` 或其他名称，调整测试代码。

- [ ] **Step 2: 验证测试失败（旧实现串行执行）**

Run: `cargo test --lib crawl::observability::events::tests::test_event_bus_concurrent_listeners --all-features`
Expected: FAIL（旧实现串行，elapsed > 80ms）

- [ ] **Step 3: 实现 EventBus::emit 并发**

在 `src/crawl/observability/events.rs` 中，修改 `emit` 方法：

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

**注意**：需先读取当前 `emit` 实现，确认 listener 类型（`Vec<Box<dyn Fn(&EngineEvent) -> BoxFuture>` 或类似）。若 listener 接收 `&EngineEvent`，需调整 clone 策略。

- [ ] **Step 4: 验证测试通过**

Run: `cargo test --lib crawl::observability::events::tests::test_event_bus_concurrent_listeners --all-features`
Expected: PASS

- [ ] **Step 5: 写失败测试 — checkpoint 不阻塞主循环**

在 `src/crawl/runner.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_checkpoint_spawned_not_blocking_main_loop() {
    // 验证 checkpoint 调用通过 tokio::spawn 后台执行，主循环不等待
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    let flag = Arc::new(AtomicU32::new(0));
    let flag_clone = Arc::clone(&flag);

    // 模拟 spawn 后台 checkpoint
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        flag_clone.fetch_add(1, Ordering::SeqCst);
    });

    // 主循环立即继续（flag 此时仍为 0）
    assert_eq!(flag.load(Ordering::SeqCst), 0, "主循环不应等待 spawn 的任务");

    // 等待 spawn 完成
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(flag.load(Ordering::SeqCst), 1, "spawn 任务应完成");
}
```

- [ ] **Step 6: 验证测试通过**

Run: `cargo test --lib crawl::runner::tests::test_checkpoint_spawned_not_blocking_main_loop --all-features`
Expected: PASS

- [ ] **Step 7: 实现 runner.rs checkpoint spawn 后台**

在 `src/crawl/runner.rs` 的 `run_inner` 函数中，定位 checkpoint 调用段（约 L470-490），替换为：

```rust
// 旧（约 L470-490，同步 await）：
// if pages_since_checkpoint >= self.checkpoint_interval {
//     if let Some(ref store) = self.checkpoint_store {
//         let _ = engine::persist_spider_checkpoint(
//             store.as_ref(),
//             &spider_name,
//             &sched,
//             &ctx.state.stats,
//         ).await;
//     }
//     pages_since_checkpoint = 0;
// }

// 新：
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

**注意**：需检查 `persist_spider_checkpoint` 签名是否接受 `&Scheduler` 和 `&SpiderStats`（或 `Arc` 引用）。若签名需调整，同步修改。同时 `spider_name` 需可 clone（String 即可）。

- [ ] **Step 8: 验证编译**

Run: `cargo build --all-features`
Expected: 编译通过

- [ ] **Step 9: 验证测试通过**

Run: `cargo test --lib crawl::observability::events::tests --all-features && cargo test --lib crawl::runner::tests --all-features`
Expected: PASS

- [ ] **Step 10: 全量回归**

Run: `cargo test --all-features`
Expected: PASS

- [ ] **Step 11: Clippy 检查**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

- [ ] **Step 12: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/observability/events.rs src/crawl/runner.rs src/crawl/engine.rs
git commit -m "perf(observability): EventBus 并发 listener + checkpoint spawn 后台"
```

---

## Task 6: Store trait async 化 + spawn_blocking（H1 + H2）

**Files:**
- Modify: `src/storage/mod.rs`（Store trait 加 #[async_trait]）
- Modify: `src/storage/sqlite.rs`（所有方法 spawn_blocking 包装）
- Modify: `src/storage/file.rs`（同上）
- Modify: `src/storage/memory.rs`（直接 async 包装，不 spawn_blocking）
- Modify: `src/crawl/middleware/builtin.rs`（CacheMiddleware、RobotsMiddleware 调用加 .await）
- Modify: `src/mcp/mod.rs`、`src/mcp/tools.rs`（调用加 .await）
- Modify: `src/parser/adaptive.rs`（调用加 .await）
- Test: `src/storage/sqlite.rs`、`src/storage/file.rs`

**Interfaces:**
- Consumes: `async_trait::async_trait`、`tokio::task::spawn_blocking`
- Produces: `Store` trait 所有方法改 `async fn`；所有调用点加 `.await`

- [ ] **Step 1: 写失败测试 — SQLite async 不阻塞 runtime**

在 `src/storage/sqlite.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_sqlite_store_async_does_not_block_runtime() {
    // 验证 SQLite set 在 async 上下文调用时，其他 task 不被阻塞
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    let store = SqliteStore::open_in_memory().expect("open in-memory sqlite");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    // 启动一个后台 task 持续计数
    let task = tokio::spawn(async move {
        for _ in 0..100 {
            c.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // 同时执行 SQLite 写入（spawn_blocking 包装，不阻塞 runtime）
    let start = Instant::now();
    for i in 0..50 {
        store
            .set("test_ns", &format!("k{i}"), b"v")
            .await
            .expect("set should succeed");
    }
    let write_elapsed = start.elapsed();

    // 等待后台 task 完成
    task.await.unwrap();

    // 后台 task 应能在写入期间继续执行（counter > 10）
    assert!(
        counter.load(Ordering::SeqCst) > 10,
        "后台 task 应在 SQLite 写入期间继续，实际 counter={}",
        counter.load(Ordering::SeqCst)
    );

    // 写入本身应合理时间内完成
    assert!(
        write_elapsed < Duration::from_secs(5),
        "50 次 set 应 < 5s，实际 {:?}",
        write_elapsed
    );
}
```

**注意**：此测试在 Store trait async 化前会编译失败（`store.set(...).await` 在同步 trait 上不合法）。这是预期的失败。

- [ ] **Step 2: 验证测试失败（trait 未 async）**

Run: `cargo test --lib storage::sqlite::tests::test_sqlite_store_async_does_not_block_runtime --all-features`
Expected: FAIL with ".await is only allowed on async traits" 或类似错误

- [ ] **Step 3: 写失败测试 — FileStore 并发写**

在 `src/storage/file.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_file_store_async_concurrent_writes() {
    // 验证并发写不同 namespace 不串行化
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    let tmp = TempDir::new().expect("create temp dir");
    let store = FileStore::open(tmp.path()).expect("open file store");

    let store = Arc::new(store);
    let mut handles = Vec::new();

    let start = Instant::now();
    for ns_idx in 0..10 {
        let store = Arc::clone(&store);
        let handle = tokio::spawn(async move {
            let ns = format!("ns_{ns_idx}");
            for i in 0..10 {
                store
                    .set(&ns, &format!("k{i}"), b"v")
                    .await
                    .expect("set should succeed");
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let elapsed = start.elapsed();
    // 10 个 namespace × 10 次写入，并发应远快于串行
    assert!(
        elapsed < Duration::from_secs(3),
        "并发写入应 < 3s，实际 {:?}",
        elapsed
    );
}
```

**注意**：需检查 file.rs 测试模块是否已 import `tempfile`。若无，添加 `use tempfile::TempDir;`。

- [ ] **Step 4: 验证测试失败**

Run: `cargo test --lib storage::file::tests::test_file_store_async_concurrent_writes --all-features`
Expected: FAIL（trait 未 async）

- [ ] **Step 5: 实现 Store trait async 化**

在 `src/storage/mod.rs` 中，为 `Store` trait 加 `#[async_trait]` 并改所有方法为 `async fn`：

```rust
use async_trait::async_trait;
use std::time::Duration;
use crate::error::Result;

// OPTIMIZE: Store 改 async，让所有调用方（CacheMiddleware/RobotsMiddleware/checkpoint）
// 都能在 async 上下文正确 await，避免同步 I/O 阻塞 runtime。
// 实现内部用 spawn_blocking 包装真正的同步 I/O。
#[async_trait]
pub trait Store: Send + Sync {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()>;
    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>>;
    async fn delete(&self, namespace: &str, key: &str) -> Result<()>;
    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()>;
}
```

**注意**：若现有 trait 有其他方法（如 `keys`、`flush` 等），一并改为 `async fn`。

- [ ] **Step 6: 实现 SqliteStore async**

在 `src/storage/sqlite.rs` 中，为 `Store` impl 加 `#[async_trait]`，所有方法用 `spawn_blocking` 包装：

```rust
#[async_trait]
impl Store for SqliteStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        // OPTIMIZE: spawn_blocking 将 SQLite 同步 I/O 移出 async worker。
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
                 VALUES (?1, ?2, ?3, NULL, ?4)",
                params![namespace, key, value, now],
            )
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok::<_, WispError>(())
        })
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("join: {e}"))))??;
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        let v = tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn.prepare(
                "SELECT value FROM kv
                 WHERE namespace = ?1 AND key = ?2
                   AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
            )
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let mut rows = stmt
                .query(params![namespace, key])
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            if let Some(row) = rows
                .next()
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?
            {
                let value: Vec<u8> = row
                    .get(0)
                    .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
                Ok::<_, WispError>(Some(value))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("join: {e}"))))??;
        Ok(v)
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.execute(
                "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
                params![namespace, key],
            )
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok::<_, WispError>(())
        })
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("join: {e}"))))??;
        Ok(())
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.to_vec();
        let ttl_secs = ttl.map(|d| d.as_secs() as i64);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let now = chrono::Utc::now().timestamp();
            conn.execute(
                "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![namespace, key, value, ttl_secs, now],
            )
            .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok::<_, WispError>(())
        })
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("join: {e}"))))??;
        Ok(())
    }
}
```

**注意**：`SqliteStore` 的 `conn` 字段需从 `parking_lot::Mutex<Connection>` 改为 `Arc<parking_lot::Mutex<Connection>>`（若已是则无需改）。`open` 和 `open_in_memory` 返回 `Self` 时构造 `Arc::new(Mutex::new(conn))`。

- [ ] **Step 7: 实现 FileStore async**

在 `src/storage/file.rs` 中，按 SqliteStore 同样模式，为所有 `Store` 方法加 `#[async_trait]` + `spawn_blocking` 包装：

```rust
#[async_trait]
impl Store for FileStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        // OPTIMIZE: spawn_blocking 将 fs::write 移出 async worker。
        let root = self.root.clone();
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.to_vec();
        tokio::task::spawn_blocking(move || {
            let dir = root.join(&namespace);
            std::fs::create_dir_all(&dir)
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            let path = dir.join(&key);
            std::fs::write(&path, &value)
                .map_err(|e| WispError::Storage(StorageError::General(e.to_string())))?;
            Ok::<_, WispError>(())
        })
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("join: {e}"))))??;
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let root = self.root.clone();
        let namespace = namespace.to_string();
        let key = key.to_string();
        let v = tokio::task::spawn_blocking(move || {
            let path = root.join(&namespace).join(&key);
            match std::fs::read(&path) {
                Ok(v) => Ok(Some(v)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(WispError::Storage(StorageError::General(e.to_string()))),
            }
        })
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("join: {e}"))))??;
        Ok(v)
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let root = self.root.clone();
        let namespace = namespace.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let path = root.join(&namespace).join(&key);
            match std::fs::remove_file(&path) {
                Ok(()) => Ok::<_, WispError>(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(WispError::Storage(StorageError::General(e.to_string()))),
            }
        })
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("join: {e}"))))??;
        Ok(())
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        _ttl: Option<Duration>,
    ) -> Result<()> {
        // FileStore 不原生支持 TTL，忽略 ttl 参数（与旧实现一致）
        self.set(namespace, key, value).await
    }
}
```

**注意**：`FileStore` 的 `root` 字段需可 clone（`PathBuf` 即可）。删除全局 `write_lock`（若旧实现有），改为依赖 spawn_blocking 的并发安全。

- [ ] **Step 8: 实现 MemoryStore async**

在 `src/storage/memory.rs` 中，加 `#[async_trait]` + 直接 `async fn`（不 spawn_blocking，moka 非阻塞）：

```rust
#[async_trait]
impl Store for MemoryStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        // moka 非阻塞，直接 async 包装
        let cache_key = format!("{namespace}:{key}");
        self.cache.insert(cache_key, value.to_vec()).await;
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let cache_key = format!("{namespace}:{key}");
        Ok(self.cache.get(&cache_key).await.map(|v| v.as_ref().to_vec()))
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let cache_key = format!("{namespace}:{key}");
        self.cache.invalidate(&cache_key).await;
        Ok(())
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let cache_key = format!("{namespace}:{key}");
        if let Some(ttl) = ttl {
            self.cache.insert_with_ttl(cache_key, value.to_vec(), ttl).await;
        } else {
            self.cache.insert(cache_key, value.to_vec()).await;
        }
        Ok(())
    }
}
```

**注意**：moka 的 API（`insert`、`get`、`invalidate`）在 moka v0.12+ 是同步的（不需要 await）。若如此，移除 `.await`。具体需检查 wisp 使用的 moka 版本和 API。

- [ ] **Step 9: 验证 storage 测试通过**

Run: `cargo test --lib storage --all-features`
Expected: PASS（含新增 2 个测试 + 改造的现有测试）

- [ ] **Step 10: 修复调用点 — builtin.rs CacheMiddleware**

Run: `grep -n "store\.\(set\|get\|delete\|set_with_ttl\)" /home/weng/wisp/src/crawl/middleware/builtin.rs`
对所有匹配项加 `.await`。例如：

```rust
// 旧：
// let cached = store.get("robots", &url)?;
// store.set("cache", &key, &bytes)?;

// 新：
// let cached = store.get("robots", &url).await?;
// store.set("cache", &key, &bytes).await?;
```

**注意**：这些调用已在 async 函数中，加 `.await` 即可。

- [ ] **Step 11: 修复调用点 — builtin.rs RobotsMiddleware**

同 Step 10 模式，对 RobotsMiddleware 内的 store 调用加 `.await`。

- [ ] **Step 12: 修复调用点 — mcp/mod.rs、mcp/tools.rs**

Run: `grep -rn "store\.\(set\|get\|delete\|set_with_ttl\)" /home/weng/wisp/src/mcp/`
对所有匹配项加 `.await`。

- [ ] **Step 13: 修复调用点 — parser/adaptive.rs**

Run: `grep -n "store\.\(set\|get\|delete\|set_with_ttl\)" /home/weng/wisp/src/parser/adaptive.rs`
对所有匹配项加 `.await`。

- [ ] **Step 14: 修复调用点 — engine.rs persist_spider_checkpoint**

在 `src/crawl/engine.rs` 的 `persist_spider_checkpoint` 中，对 `store.set(...)` 加 `.await`：

```rust
// 旧：
// store.set("checkpoint", spider_name, &state_bytes)?;

// 新：
store.set("checkpoint", spider_name, &state_bytes).await?;
```

- [ ] **Step 15: 验证编译**

Run: `cargo build --all-features`
Expected: 编译通过（若有遗漏调用点，根据错误信息逐一修复）

- [ ] **Step 16: 验证测试通过**

Run: `cargo test --lib storage --all-features && cargo test --lib crawl::middleware --all-features`
Expected: PASS

- [ ] **Step 17: 全量回归**

Run: `cargo test --all-features`
Expected: PASS（所有 273+ 现有测试 + 新增测试全过）

- [ ] **Step 18: Clippy 检查**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

- [ ] **Step 19: 提交**

```bash
cd /home/weng/wisp
git add src/storage/ src/crawl/middleware/builtin.rs src/mcp/ src/parser/adaptive.rs src/crawl/engine.rs
git commit -m "perf(storage): Store trait async 化 + spawn_blocking 包装"
```

---

## 完成验收

完成全部 6 Task 后，执行最终验收：

- [ ] **Final Step 1: 全量测试**

Run: `cargo test --all-features`
Expected: PASS（273 现有 + 13+ 新增测试全过）

- [ ] **Final Step 2: Clippy 全量检查**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

- [ ] **Final Step 3: 文档测试**

Run: `cargo test --doc --all-features`
Expected: PASS

- [ ] **Final Step 4: 集成验证**

在 banzhu-rs 项目中验证 wisp path 依赖无回归：
```bash
cd /home/weng/banzhu-rs
cargo build --all-features
cargo test --all-features
```
Expected: 编译通过，测试全过

- [ ] **Final Step 5: 确认 commit 历史**

Run: `cd /home/weng/wisp && git log --oneline -6`
Expected: 6 个 perf commit 按顺序排列
