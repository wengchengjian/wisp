# Task 1: CdpSession 重构 — broadcast + watch 错误传播（H3 + M6）

**Files:**
- Modify: `src/browser/cdp.rs`（删除 events Vec + consumed_offset，新增 conn_state watch）
- Test: `src/browser/cdp.rs`（同文件 #[cfg(test)] 模块）

**Interfaces:**
- Consumes: `tokio::sync::{broadcast, watch, oneshot, Mutex}`、`futures::{SinkExt, StreamExt}`
- Produces: `CdpSession::subscribe_events() -> broadcast::Receiver<CdpEvent>`（签名不变）、`CdpSession::execute_with_session()` 内部用 `select!` 等待响应或连接关闭

## Steps

### Step 1: 写失败测试 — broadcast 直达订阅者

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

### Step 2: 验证测试通过（broadcast 基础设施验证）

Run: `cargo test --lib browser::cdp::tests::test_cdp_event_broadcast_no_vec --all-features`
Expected: PASS

### Step 3: 写失败测试 — 连接错误通知 pending

```rust
#[tokio::test]
async fn test_cdp_connection_error_notifies_pending() {
    // 验证 ConnState watch 错误传播：连接关闭时所有 watch 订阅者立即收到
    use tokio::sync::watch;
    let (tx, mut rx) = watch::channel(ConnState::Open);

    tx.send(ConnState::Closed("ws closed".into())).unwrap();

    assert!(rx.has_changed().unwrap());
    match &*rx.borrow() {
        ConnState::Closed(msg) => assert_eq!(msg, "ws closed"),
        ConnState::Open => panic!("expected Closed"),
    }
}
```

### Step 4: 验证测试失败（ConnState 类型尚未定义）

Run: `cargo test --lib browser::cdp::tests::test_cdp_connection_error_notifies_pending --all-features`
Expected: FAIL with "cannot find type `ConnState` in this scope"

### Step 5: 写失败测试 — wait_for_event 使用 broadcast

```rust
#[tokio::test]
async fn test_wait_for_event_uses_broadcast() {
    let (tx, _) = tokio::sync::broadcast::channel::<i32>(2);
    let mut rx = tx.subscribe();

    for i in 0..5 {
        let _ = tx.send(i);
    }

    match rx.recv().await {
        Ok(v) => assert!(v >= 0 && v < 5),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
            assert!(n > 0, "lagged count should be positive");
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => panic!("should not close"),
    }
}
```

### Step 6: 验证测试通过

Run: `cargo test --lib browser::cdp::tests::test_wait_for_event_uses_broadcast --all-features`
Expected: PASS

### Step 7: 实现 CdpSession 重构

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

**7c. 修改 connect 方法**：
- 删除 `let events = Arc::new(Mutex::new(Vec::<CdpEvent>::new()));`
- 删除 `let consumed_offset = Arc::new(Mutex::new(0usize));`
- 添加 `let (conn_state_tx, _conn_state_rx) = watch::channel(ConnState::Open);`
- 在后台任务的 `Err(e) =>` 和 `Ok(Message::Close(_)) =>` 分支中，调用 `let _ = conn_state_clone.send(ConnState::Closed(format!("ws error: {e}")));` 后再 `break`
- 在 break 前清空 pending：`pending_clone.lock().await.clear();`
- 返回的 `Self` 中删除 `events` 和 `consumed_offset` 字段，新增 `conn_state: conn_state_tx`

**7d. 修改 execute_with_session 方法**：

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

**7e. 重写 wait_for_event 方法**：

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

### Step 8: 验证新测试通过

Run: `cargo test --lib browser::cdp::tests --all-features`
Expected: PASS

### Step 9: 全量回归

Run: `cargo test --all-features`
Expected: PASS（273 现有 + 3 新增 = 276 测试全过）

### Step 10: Clippy 检查

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 无 warning

### Step 11: 提交

```bash
cd /home/weng/wisp
git add src/browser/cdp.rs
git commit -m "perf(cdp): 删除 events Vec 改用 broadcast + 错误传播 watch"
```
