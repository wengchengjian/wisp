# Task 3: follow_rx Mutex 移除（H5）

**Files:**
- Modify: `src/crawl/engine.rs`（删除 EngineShared.follow_rx 字段 + 4 处测试构造）
- Modify: `src/crawl/runner.rs`（unfold 状态持有 Receiver + 主构造）
- Modify: `src/crawl/builder.rs`（若 EngineShared 构造涉及 follow_rx）
- Test: `src/crawl/engine.rs`、`src/crawl/runner.rs`

**Interfaces:**
- Consumes: `tokio::sync::mpsc::UnboundedReceiver`
- Produces: `EngineShared` 不再有 `follow_rx` 字段；`run_inner` 内部 unfold 状态持有 `UnboundedReceiver<Request>`

## Steps

### Step 1: 写失败测试 — follow_rx drain 无 Mutex

在 `src/crawl/runner.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[tokio::test]
async fn test_follow_rx_drained_without_mutex() {
    // 验证 UnboundedReceiver 可直接 try_recv drain，无需 Mutex 包装
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<i32>();

    tx.send(1).unwrap();
    tx.send(2).unwrap();
    tx.send(3).unwrap();

    let mut drained = Vec::new();
    while let Ok(v) = rx.try_recv() {
        drained.push(v);
    }

    assert_eq!(drained, vec![1, 2, 3]);
}
```

### Step 2: 验证测试通过

Run: `cargo test --lib crawl::runner::tests::test_follow_rx_drained_without_mutex --all-features`
Expected: PASS

### Step 3: 删除 EngineShared.follow_rx 字段

**3a. 删除 EngineShared 结构体的 `follow_rx` 字段**：

```rust
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

**3b. 删除所有测试构造中的 follow_rx 初始化**：

每处都删除 `follow_rx: Arc::new(Mutex::new(follow_rx)),` 行。同时可删除 `let (follow_tx, follow_rx) = mpsc::unbounded_channel::<Request>();` 中的 `follow_rx` 绑定（若测试不需要单独持有 receiver），改为 `let (follow_tx, _) = mpsc::unbounded_channel::<Request>();`。

### Step 4: 修改 runner.rs — unfold 状态持有 Receiver

**4a. 修改 unfold 状态类型**（包含 `UnboundedReceiver<Request>`）：

```rust
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

**4b. 删除 `let mut rx_guard = ctx.shared.follow_rx.lock().await;` 及 `drop(rx_guard);`**

### Step 5: 检查 builder.rs

Run: `grep -n "follow_rx" /home/weng/wisp/src/crawl/builder.rs`
若有匹配，按相同模式删除 follow_rx 构造。

### Step 6: 验证编译

Run: `cargo build --all-features`
Expected: 编译通过

### Step 7: 验证测试通过

Run: `cargo test --lib crawl::runner::tests::test_follow_rx_drained_without_mutex --all-features`
Expected: PASS

### Step 8: 全量回归

Run: `cargo test --all-features`
Expected: PASS（pre-existing 失败除外）

### Step 9: Clippy 检查

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: 修改文件无新 warning

### Step 10: 提交

```bash
cd /home/weng/wisp
git add src/crawl/engine.rs src/crawl/runner.rs src/crawl/builder.rs
git commit -m "perf(runner): follow_rx 移入 unfold 状态移除 Mutex"
```
