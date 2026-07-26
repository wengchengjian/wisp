# Turso 替换 + Clippy 修复 + 剩余性能优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 wisp 框架异步/并发性能审查报告中剩余的优化项：用 turso 替换 rusqlite、消除 601 个 clippy 警告、处理 M8/M9/M10 + L1-L5/L7 共 9 处局部优化、清理死代码 SessionPool。

**Architecture:** 8 个独立任务并行推进，每个任务独立 commit + 全量回归。任务顺序：从最简单的死代码清理开始，到最复杂的 Turso 替换结束，Clippy 修复放在最后（避免前面改动引入新警告被反复修复）。

**Tech Stack:** Rust + Tokio + turso 0.7 + parking_lot + moka + async-trait + tracing

## Global Constraints

- **分支**：master 主分支直接开发，禁止 worktree/feature branch
- **兼容性**：不向后兼容，删除旧字段/方法不保留兼容层
- **测试基线**：现有测试（约 439 个）必须每任务后全过
- **依赖**：仅引入 turso（替换 rusqlite），其他不新增
- **保留 `feature = "sqlite"`**：turso 在 sqlite feature gate 后面，与 MemoryStore/FileStore 并列关系不变
- **提交信息**：中文简短一行，格式 `perf(<scope>): <描述>` 或 `fix(<scope>): <描述>` 或 `chore(<scope>): <描述)`
- **验证命令**：`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings`
- **工作目录**：`/home/weng/wisp`
- **turso API 已确认**（基于 docs.rs/turso/latest）：
  - `turso::Builder::new_local(path: &str).build().await -> Result<Database>`
  - `db.connect() -> Connection`（同步方法）
  - `conn.execute(sql, params).await -> Result<u64>`
  - `conn.query(sql, params).await -> Result<Rows>`
  - `conn.execute_batch(sql).await -> Result<()>`
  - `conn.prepare(sql).await -> Result<Statement>`
  - `stmt.query(params).await -> Result<Rows>`
  - `rows.next().await -> Option<Result<Row>>`
  - `row.get_value(idx) -> Result<Value>`
  - `turso::Value` 变体：`Null`、`Integer(i64)`、`Real(f64)`、`Text(String)`、`Blob(Vec<u8>)`
  - `turso::params![v1, v2, ...]` 宏（与 rusqlite 一致）

## File Structure

| Task | 文件 | 责任 |
|---|---|---|
| 1 | `src/crawl/runtime/session_pool.rs`, `src/crawl/runtime/mod.rs` | 删除死代码 SessionPool |
| 2 | `src/fetcher/client.rs` | M10 println! 改 tracing::trace! |
| 3 | `src/crawl/scheduling/scheduler.rs` | M9 parking_lot::Mutex 替换 tokio::sync::Mutex |
| 4 | `src/crawl/middleware/builtin.rs` | M8 score_body 大 body spawn_blocking |
| 5 | `src/crawl/middleware/builtin.rs`, `src/proxy.rs`, `src/crawl/engine.rs`, `src/browser/cdp.rs` | L1/L2/L3/L4/L7 小优化 |
| 6 | `src/crawl/observability/events.rs` | L5 EventListener 改 Arc<EngineEvent> |
| 7 | `src/storage/sqlite.rs`, `Cargo.toml`, `tests/*`, `src/mcp/*` | Turso 替换 rusqlite |
| 8 | 全项目 | Clippy 自动修复 + code review |

---

## Task 1: 删除死代码 SessionPool

**Files:**
- Modify: `src/crawl/runtime/mod.rs`（删除 `pub mod session_pool;` 声明）
- Delete: `src/crawl/runtime/session_pool.rs`

**Interfaces:**
- Consumes: 无
- Produces: 无（纯删除）

- [ ] **Step 1: 确认 SessionPool 无引用**

Run: `cd /home/weng/wisp && grep -rn "session_pool\|SessionPool" src/ tests/ --include="*.rs"`
Expected: 只在 `src/crawl/runtime/session_pool.rs` 和 `src/crawl/runtime/mod.rs` 出现，无其他引用

- [ ] **Step 2: 删除 session_pool.rs 文件**

```bash
rm /home/weng/wisp/src/crawl/runtime/session_pool.rs
```

- [ ] **Step 3: 修改 runtime/mod.rs**

读取 `src/crawl/runtime/mod.rs`，如果只有 `pub mod session_pool;` 一行，删除整个文件；如果还有其他模块声明，只删除 `pub mod session_pool;` 行。

```bash
cat /home/weng/wisp/src/crawl/runtime/mod.rs
```

如果文件内容只有 `pub mod session_pool;`：
```bash
rm /home/weng/wisp/src/crawl/runtime/mod.rs
rmdir /home/weng/wisp/src/crawl/runtime  # 如果目录为空
```

如果还有其他模块，编辑文件删除 `pub mod session_pool;` 行。

- [ ] **Step 4: 检查 crawl/mod.rs**

```bash
grep -n "pub mod runtime" /home/weng/wisp/src/crawl/mod.rs
```

如果 runtime 目录被删除，相应删除 `pub mod runtime;` 声明。

- [ ] **Step 5: 编译验证**

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

- [ ] **Step 6: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 7: Clippy 检查**

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20`
Expected: 不增加新警告（已有的 601 个警告保持不变）

- [ ] **Step 8: 提交**

```bash
cd /home/weng/wisp
git add -A
git commit -m "chore(runtime): 删除死代码 SessionPool"
```

---

## Task 2: M10 println! 改 tracing::trace!

**Files:**
- Modify: `src/fetcher/client.rs`（`do_browser_work_inner` 中的 `println!("[timing] ...")`）

**Interfaces:**
- Consumes: `tracing::trace!`、`crate::utils::sanitize_url`
- Produces: 无 API 变化

- [ ] **Step 1: 定位所有 println!**

Run: `cd /home/weng/wisp && grep -n "println!" src/fetcher/client.rs`
Expected: 找到所有 `println!("[timing] ...")` 行

- [ ] **Step 2: 替换为 tracing::trace!**

修改 `src/fetcher/client.rs` 中 `do_browser_work_inner` 函数，把所有 `println!("[timing] ...")` 改为 `tracing::trace!`。

例如：
```rust
// 旧
println!("[timing] goto+recv: {:?} ({})", elapsed, url);

// 新
tracing::trace!(elapsed = ?elapsed, url = %sanitize_url(&url), "goto+recv timing");
```

注意：
- 用 `trace` 级别（不是 `info` 或 `debug`），因为 timing 是调试用
- URL 必须用 `sanitize_url` 脱敏（避免日志泄露凭据）
- 保留原有的 timing 信息（elapsed、url 等）

- [ ] **Step 3: 编译验证**

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

- [ ] **Step 4: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 5: 提交**

```bash
cd /home/weng/wisp
git add src/fetcher/client.rs
git commit -m "perf(fetcher): println! 改 tracing::trace! 避免 stdout 阻塞"
```

---

## Task 3: M9 Scheduler parking_lot::Mutex 替换 tokio::sync::Mutex

**Files:**
- Modify: `src/crawl/scheduling/scheduler.rs`

**Interfaces:**
- Consumes: `parking_lot::Mutex`
- Produces: 无 API 变化（`async fn push/pop` 签名不变）

- [ ] **Step 1: 读取当前 Scheduler 实现**

Run: `cd /home/weng/wisp && sed -n '1,50p' src/crawl/scheduling/scheduler.rs`
Expected: 看到 `heap: Arc<tokio::sync::Mutex<HeapInner>>` 字段

- [ ] **Step 2: 修改 Scheduler 结构体**

在 `src/crawl/scheduling/scheduler.rs` 中：

**2a. 修改 import：**

```rust
// 旧
use tokio::sync::Mutex;

// 新
use parking_lot::Mutex as PlMutex;
```

**2b. 修改 Scheduler 结构体的 heap 字段类型：**

```rust
// 旧
pub struct Scheduler {
    heap: Arc<Mutex<HeapInner>>,
    // ...
}

// 新（添加注释说明不跨 await 持锁）
/// 注意：`heap` 用 `parking_lot::Mutex`（同步锁），临界区内禁止 `.await`。
/// 如需在持锁期间 await，必须改回 `tokio::sync::Mutex` 或重构。
pub struct Scheduler {
    heap: Arc<PlMutex<HeapInner>>,
    // ...
}
```

**2c. 修改 push 方法：**

```rust
// 旧
pub async fn push(&self, req: Request) {
    // ...
    let mut g = self.heap.lock().await;
    // ...
}

// 新（去掉 .await）
pub async fn push(&self, req: Request) {
    // ...
    let mut g = self.heap.lock();
    // ...
}
```

**2d. 修改 pop 方法：**

```rust
// 旧
pub async fn pop(&self) -> Option<Request> {
    let mut g = self.heap.lock().await;
    // ...
}

// 新
pub async fn pop(&self) -> Option<Request> {
    let mut g = self.heap.lock();
    // ...
}
```

**2e. 修改其他用到 `self.heap.lock().await` 的方法**（如 `len`、`is_empty` 等）：

搜索所有 `.lock().await` 替换为 `.lock()`：
```bash
grep -n "heap.lock().await" /home/weng/wisp/src/crawl/scheduling/scheduler.rs
```

逐个修改为 `self.heap.lock()`（无 `.await`）。

- [ ] **Step 3: 检查 pending_urls 和 seen_urls 字段**

```bash
grep -n "Mutex\|RwLock" /home/weng/wisp/src/crawl/scheduling/scheduler.rs
```

如果 `pending_urls` 或 `seen_urls` 也用 `tokio::sync::Mutex` 且内部无 await，同样改为 `parking_lot::Mutex`。

注意：`DashSet` / `DashMap` 无锁，不需要改。

- [ ] **Step 4: 编译验证**

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

如果出现 `parking_lot::Mutex` 不能跨 await 持锁的错误，说明临界区内有 await，需要重构（提取同步部分到局部变量，await 在 lock 释放后执行）。

- [ ] **Step 5: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 6: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/scheduling/scheduler.rs
git commit -m "perf(scheduler): tokio::sync::Mutex 改 parking_lot::Mutex 消除 yield 开销"
```

---

## Task 4: M8 score_body 大 body spawn_blocking

**Files:**
- Modify: `src/crawl/middleware/builtin.rs`（`DynamicUpgradeMiddleware::score_body` 或类似函数）

**Interfaces:**
- Consumes: `tokio::task::spawn_blocking`、`aho_corasick::AhoCorasick`（已 Clone + Send + Sync）
- Produces: 无 API 变化（`score_body` 内部行为变更）

- [ ] **Step 1: 定位 score_body 函数**

Run: `cd /home/weng/wisp && grep -n "fn score_body\|fn score" src/crawl/middleware/builtin.rs`
Expected: 找到 `score_body` 或类似函数定义

- [ ] **Step 2: 读取当前实现**

读取 `score_body` 函数完整实现，理解 aho-corasick 匹配 + HTML 解析逻辑。

- [ ] **Step 3: 写失败测试 — 大 body 不阻塞 runtime**

在 `src/crawl/middleware/builtin.rs` 的 `#[cfg(test)]` 模块追加：

```rust
/// Task 4：验证大 body（>1MB）score_body 不阻塞 tokio runtime。
///
/// OPTIMIZE: 旧实现直接在 async worker 上执行 aho-corasick + HTML 解析，
/// 大 body（5MB）会阻塞 worker 数十毫秒。
/// 改用 spawn_blocking 后，大 body 移到 blocking pool，runtime 不阻塞。
#[tokio::test]
async fn test_score_body_large_does_not_block_runtime() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    // 构造一个 2MB 的 HTML body（> 1MB 阈值）
    let large_body: Vec<u8> = b"<html><body>".repeat(200_000); // ~2.4MB

    let middleware = DynamicUpgradeMiddleware::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    // 后台 task：每 1ms 自增 counter
    let task = tokio::spawn(async move {
        for _ in 0..100 {
            c.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // 同时执行 score_body（应 spawn_blocking，不阻塞 runtime）
    let start = Instant::now();
    let _score = middleware.score_body(large_body).await;
    let elapsed = start.elapsed();

    task.await.unwrap();

    let counter_val = counter.load(Ordering::SeqCst);
    // 后台 task 应在 score_body 期间继续推进（spawn_blocking 不占用 worker）
    assert!(
        counter_val > 50,
        "后台 task 应在 score_body 期间继续，实际 counter={counter_val}"
    );
    // score_body 应 < 2s（spawn_blocking 不阻塞）
    assert!(
        elapsed < Duration::from_secs(2),
        "score_body 应 < 2s，实际 {elapsed:?}"
    );
}
```

- [ ] **Step 4: 验证测试失败（如果 score_body 当前不 spawn_blocking）**

Run: `cd /home/weng/wisp && cargo test --lib crawl::middleware::builtin::tests::test_score_body_large_does_not_block_runtime --all-features`
Expected: FAIL（counter < 50，因为当前 score_body 直接在 worker 上执行阻塞了 runtime）

如果测试通过，说明 score_body 已经是 spawn_blocking 或 body 不够大触发不了阻塞，需要调整 body 大小或阈值。

- [ ] **Step 5: 实现 score_body spawn_blocking**

修改 `score_body` 函数：

```rust
const LARGE_BODY_THRESHOLD: usize = 1 << 20; // 1MB

impl DynamicUpgradeMiddleware {
    pub async fn score_body(&self, body: Vec<u8>) -> u32 {
        if body.len() > LARGE_BODY_THRESHOLD {
            // 大 body 移到 blocking pool，避免阻塞 tokio worker
            let matcher = self.matcher.clone();  // aho-corasick AhoCorasick: Clone + Send + Sync
            tokio::task::spawn_blocking(move || {
                Self::score_body_sync(&body, &matcher)
            })
            .await
            .expect("spawn_blocking join failed")
        } else {
            // 小 body 直接同步执行（避免 spawn 开销）
            Self::score_body_sync(&body, &self.matcher)
        }
    }

    fn score_body_sync(body: &[u8], matcher: &AhoCorasick) -> u32 {
        // 原有 aho-corasick 匹配 + HTML 解析逻辑
        // 把原 score_body 函数体搬到这里，去掉 .await
        // ...
    }
}
```

注意：
- `AhoCorasick` 必须 `Clone + Send + Sync`（aho-corasick crate 满足）
- `score_body_sync` 是纯同步函数，无 await
- 保留原有的 scoring 逻辑（strong/medium/weak signals）

- [ ] **Step 6: 验证新测试通过**

Run: `cd /home/weng/wisp && cargo test --lib crawl::middleware::builtin::tests::test_score_body_large_does_not_block_runtime --all-features`
Expected: PASS

- [ ] **Step 7: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 8: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/middleware/builtin.rs
git commit -m "perf(middleware): score_body 大 body spawn_blocking 避免阻塞 runtime"
```

---

## Task 5: L1/L2/L3/L4/L7 小优化

**Files:**
- Modify: `src/crawl/middleware/builtin.rs`（L1 UaRotation &'static str、L4 score_body 短路）
- Modify: `src/proxy.rs`（L2 ProxyPool::next 返回 Arc<str>）
- Modify: `src/crawl/engine.rs`（L3 cf_domain_locks 改 moka::Cache）
- Modify: `src/browser/cdp.rs`（L7 CDP execute 预分配 buffer）

**Interfaces:**
- Consumes: `moka::Cache`（项目已有依赖）、`std::sync::Arc`
- Produces:
  - `ProxyPool::next` 返回类型从 `Option<String>` 改为 `Option<Arc<str>>`
  - `EventListener` 签名不变（L5 在 Task 6 处理）

### L1: UaRotationMiddleware 用 &'static str

- [ ] **Step 1: 定位 UaRotationMiddleware**

Run: `cd /home/weng/wisp && grep -n "UaRotation\|ua_rotation\|agents: Vec<String>" src/crawl/middleware/builtin.rs`
Expected: 找到 UaRotationMiddleware 结构体定义

- [ ] **Step 2: 修改结构体字段类型**

```rust
// 旧
pub struct UaRotationMiddleware {
    agents: Vec<String>,
    // ...
}

// 新
pub struct UaRotationMiddleware {
    agents: Vec<&'static str>,
    // ...
}
```

- [ ] **Step 3: 修改构造函数**

找到 UaRotationMiddleware 的构造函数（`new` 或 `default`），把 `Vec<String>` 改为 `Vec<&'static str>`：

```rust
// 旧
let agents: Vec<String> = vec![
    "Mozilla/5.0 ...".to_string(),
    "Mozilla/5.0 ...".to_string(),
];

// 新
let agents: Vec<&'static str> = vec![
    "Mozilla/5.0 ...",
    "Mozilla/5.0 ...",
];
```

- [ ] **Step 4: 修改使用处**

找到 UaRotationMiddleware 中使用 `self.agents` 的地方（如 `self.agents[idx].clone()`），改为直接返回 `&'static str`：

```rust
// 旧
let ua = self.agents[idx].clone();  // clone String
req.headers.insert("User-Agent", ua.parse().unwrap());

// 新
let ua = self.agents[idx];  // &'static str，无 clone
req.headers.insert("User-Agent", ua.parse().unwrap());
```

### L4: score_body 短路扫描

- [ ] **Step 5: 定位 script_count 计算**

Run: `cd /home/weng/wisp && grep -n "script_count\|find_iter.*count\|SCRIPT_DENSITY" src/crawl/middleware/builtin.rs`
Expected: 找到 `find_iter(...).count()` 全量扫描

- [ ] **Step 6: 改为短路扫描**

```rust
// 旧
let script_count = script_matcher.find_iter(body).count();

// 新（达到阈值即停）
const SCRIPT_DENSITY_THRESHOLD: usize = 6;
let script_count = script_matcher
    .find_iter(body)
    .take(SCRIPT_DENSITY_THRESHOLD)
    .count();
// 后续判断：script_count >= SCRIPT_DENSITY_THRESHOLD 即满足弱信号
```

注意：如果原逻辑是 `if script_count >= N`，改为 `if script_count >= SCRIPT_DENSITY_THRESHOLD`（N 与 take 参数一致）。

### L2: ProxyPool::next 返回 Arc<str>

- [ ] **Step 7: 定位 ProxyPool::next**

Run: `cd /home/weng/wisp && grep -n "fn next\|ProxyPool" src/proxy.rs`
Expected: 找到 `pub fn next(&self) -> Option<String>`

- [ ] **Step 8: 修改返回类型**

```rust
// 旧
pub fn next(&self) -> Option<String> {
    self.proxies.get(...).map(|s| s.clone())
}

// 新
pub fn next(&self) -> Option<Arc<str>> {
    self.proxies.get(...).cloned()  // Arc<str> clone 廉价
}
```

注意：`self.proxies` 的元素类型需要从 `String` 改为 `Arc<str>`。如果 `proxies` 是 `Vec<String>`，改为 `Vec<Arc<str>>`，构造时 `Arc::from("http://...")`。

- [ ] **Step 9: 修改调用点**

搜索所有调用 `proxy_pool.next()` 的地方：

```bash
cd /home/weng/wisp && grep -rn "\.next()" src/ tests/ --include="*.rs" | grep -i proxy
```

修改调用点：
```rust
// 旧
let proxy = proxy_pool.next();  // Option<String>
req.proxy = proxy;

// 新
let proxy = proxy_pool.next();  // Option<Arc<str>>
req.proxy = proxy.map(|p| p.to_string());  // 如果 Request::proxy 是 String
// 或保持 Arc<str>：req.proxy = proxy;（如果 Request::proxy 是 Arc<str>）
```

如果 `Request::proxy` 字段类型是 `String`，需要决定：
- 方案 A：保持 `String`，调用点 `.map(|p| p.to_string())`（仍有分配，但只在出站请求时分配，不是每次 next）
- 方案 B：改 `Request::proxy` 为 `Arc<str>`（影响面大）

**推荐方案 A**：保持 `Request::proxy` 为 `Option<String>`，调用点转换。这样 ProxyPool 内部无 clone，只在出站时分配一次。

### L3: cf_domain_locks 改 moka::Cache

- [ ] **Step 10: 定位 cf_domain_locks**

Run: `cd /home/weng/wisp && grep -n "cf_domain_locks" src/crawl/engine.rs`
Expected: 找到字段定义和使用处

- [ ] **Step 11: 修改 EngineShared 结构体**

```rust
// 旧
pub struct EngineShared {
    cf_domain_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    // ...
}

// 新（与项目已有 moka 依赖一致）
pub struct EngineShared {
    cf_domain_locks: Arc<moka::Cache<String, Arc<tokio::sync::Mutex<()>>>>,
    // ...
}
```

- [ ] **Step 12: 修改构造函数**

```rust
// 旧
cf_domain_locks: Arc::new(DashMap::new()),

// 新（与项目已有 proxy_clients 用 moka::sync::Cache 一致）
cf_domain_locks: Arc::new(
    moka::sync::Cache::builder()
        .max_capacity(1024)
        .build(),
),
```

- [ ] **Step 13: 修改使用处**

```rust
// 旧
let lock = ctx.shared.cf_domain_locks
    .entry(domain.clone())
    .or_insert_with(|| Arc::new(Mutex::new(())))
    .clone();

// 新（moka::sync::Cache::get_with 是同步方法，接收 FnOnce -> V）
let lock = ctx.shared.cf_domain_locks
    .get_with(domain.clone(), || Arc::new(tokio::sync::Mutex::new(())));
```

注意：`moka::sync::Cache::get_with` 是同步方法（不是 async），直接返回 `V`。这与项目已有的 `proxy_clients`（也是 `moka::sync::Cache`）一致。`Arc<tokio::sync::Mutex<()>>` 内部的 `Mutex` 仍然是 tokio 的（因为后续 `lock().await` 需要 async 锁）。

- [ ] **Step 14: 修改测试初始化**

搜索测试中 `cf_domain_locks: Arc::new(DashMap::new())` 改为 `cf_domain_locks: Arc::new(moka::sync::Cache::builder().max_capacity(1024).build())`：

```bash
cd /home/weng/wisp && grep -rn "cf_domain_locks:" src/ tests/ --include="*.rs"
```

逐个修改。

### L7: CDP execute 预分配 buffer

- [ ] **Step 15: 定位 execute_with_session**

Run: `cd /home/weng/wisp && grep -n "to_string.*msg\|serde_json::to_string" src/browser/cdp.rs`
Expected: 找到 `serde_json::to_string(&msg).expect("CDP msg always serializable")`

- [ ] **Step 16: 改为 to_writer 预分配 buffer**

```rust
// 旧
let text = serde_json::to_string(&msg).expect("CDP msg always serializable");
writer.send(Message::Text(text.into())).await?;

// 新（预分配 256 字节，避免 to_string 内部多次 realloc）
let mut buf = Vec::with_capacity(256);
serde_json::to_writer(&mut buf, &msg).expect("CDP msg always serializable");
let text = String::from_utf8(buf).expect("CDP msg always UTF-8");
writer.send(Message::Text(text.into())).await?;
```

- [ ] **Step 17: 编译验证**

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

- [ ] **Step 18: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 19: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/middleware/builtin.rs src/proxy.rs src/crawl/engine.rs src/browser/cdp.rs
git commit -m "perf: L1/L2/L3/L4/L7 五项小优化（UA static str/Proxy Arc<str>/cf_locks moka/短路扫描/CDP buffer）"
```

---

## Task 6: L5 EventListener 改 Arc<EngineEvent>

**Files:**
- Modify: `src/crawl/observability/events.rs`

**Interfaces:**
- Consumes: `std::sync::Arc`
- Produces:
  - `EventListener` 类型签名变更：`Fn(EngineEvent)` → `Fn(Arc<EngineEvent>)`
  - 所有 listener 注册处（`logging_listener`、`metrics_listener`、测试闭包）需更新签名

- [ ] **Step 1: 修改 EventListener 类型定义**

在 `src/crawl/observability/events.rs` 中：

```rust
// 旧
pub type EventListener = Arc<dyn Fn(EngineEvent) -> BoxFuture<'static, ()> + Send + Sync>;

// 新
pub type EventListener = Arc<dyn Fn(Arc<EngineEvent>) -> BoxFuture<'static, ()> + Send + Sync>;
```

- [ ] **Step 2: 修改 EventBus::emit**

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

- [ ] **Step 3: 修改 logging_listener**

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

// 新（Arc<EngineEvent> 自动 deref，match &event 不变）
pub fn logging_listener() -> EventListener {
    Arc::new(|event: Arc<EngineEvent>| {
        Box::pin(async move {
            match &*event {  // 解引用 Arc
                EngineEvent::CrawlStarted { spider, start_urls } => { ... }
                // ...
            }
        })
    })
}
```

注意：`match &event` 改为 `match &*event`（解引用 Arc 后再引用）。或者用 `match event.as_ref()`。

- [ ] **Step 4: 修改 metrics_listener**

```rust
// 旧
pub fn metrics_listener(metrics: Arc<Metrics>) -> EventListener {
    Arc::new(move |event: EngineEvent| {
        let metrics = Arc::clone(&metrics);
        Box::pin(async move {
            match event {  // event 是 owned EngineEvent
                EngineEvent::ResponseReceived { elapsed_ms, from_cache, .. } => { ... }
                // ...
            }
        })
    })
}

// 新（event 是 Arc<EngineEvent>，需要解引用才能 match owned）
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
                // ...
            }
        })
    })
}
```

注意：`match event`（owned）改为 `match &*event`（引用），字段 `elapsed_ms` 从 `u64` 变为 `&u64`，需要 `*elapsed_ms` 解引用。

- [ ] **Step 5: 修改测试中的 listener 闭包**

```bash
cd /home/weng/wisp && grep -n "bus.on(Arc::new" src/crawl/observability/events.rs
```

逐个修改：

```rust
// 旧
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

- [ ] **Step 6: 检查其他调用点**

```bash
cd /home/weng/wisp && grep -rn "EventListener\|bus.on\|EventBus" src/ tests/ --include="*.rs"
```

更新所有 listener 注册处的签名。

- [ ] **Step 7: 编译验证**

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

- [ ] **Step 8: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 9: 提交**

```bash
cd /home/weng/wisp
git add src/crawl/observability/events.rs
git commit -m "perf(events): EventListener 改 Arc<EngineEvent> 共享事件无 clone"
```

---

## Task 7: Turso 替换 rusqlite

**Files:**
- Modify: `Cargo.toml`（替换依赖）
- Modify: `src/storage/sqlite.rs`（重写 SqliteStore）
- Modify: `src/crawl/runner.rs`（如有 SqliteStore 构造，加 `.await`）
- Modify: `src/mcp/*`（如有 SqliteStore 引用，加 `.await`）
- Modify: `tests/crawl_e2e_real_test.rs`、`tests/crawl_cache_real_test.rs`、`tests/run_inner_test.rs`、`tests/crawl_checkpoint_test.rs`（加 `.await`）
- Modify: `src/storage/sqlite.rs` tests（`make_store` 改 async）

**Interfaces:**
- Consumes: `turso::{Builder, Database, Value as TursoValue}`、`turso::params!`
- Produces:
  - `SqliteStore::open` 签名从 `pub fn open(path: &Path) -> Result<Self>` 改为 `pub async fn open(path: &Path) -> Result<Self>`
  - `SqliteStore::open_in_memory` 同上改 async

- [ ] **Step 1: 修改 Cargo.toml**

```toml
# 旧
rusqlite = { version = "0.x", features = ["bundled"], optional = true }

# 新
turso = { version = "0.7", optional = true }

# [features] 部分
# 旧：sqlite = ["dep:rusqlite"]
# 新：sqlite = ["dep:turso"]
```

注意：保留 `sqlite = ["dep:turso"]`，feature gate 不变。

- [ ] **Step 2: 重写 src/storage/sqlite.rs**

完整重写 `src/storage/sqlite.rs`：

```rust
//! SQLite 存储后端（基于 turso，原生 async）。单表 KV 结构。
//!
//! turso 内部管理连接池，每次操作 `db.connect()` 取独立 Connection，无需手动加锁。
//! 所有 Store 方法直接 `.await`，不需要 `spawn_blocking`。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use turso::{Builder, Database, Value as TursoValue};

use crate::error::{Result, WispError, StorageError};
use super::Store;

/// SQLite 存储后端。线程安全（turso `Database` 内部管理连接池）。
pub struct SqliteStore {
    db: Database,
}

impl SqliteStore {
    /// 打开或创建数据库文件。
    pub async fn open(path: &Path) -> Result<Self> {
        let path_str = path.to_str().ok_or_else(|| {
            WispError::Storage(StorageError::General("invalid path: non-UTF8".into()))
        })?;
        let db = Builder::new_local(path_str)
            .build()
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso open: {e}"))))?;
        let store = Self { db };
        store.init_schema().await?;
        Ok(store)
    }

    /// 内存数据库（测试用）。
    pub async fn open_in_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso open in-memory: {e}"))))?;
        let store = Self { db };
        store.init_schema().await?;
        Ok(store)
    }

    async fn init_schema(&self) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("PRAGMA journal_mode: {e}"))))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("PRAGMA synchronous: {e}"))))?;

        // 旧 schema 检测：如果存在旧三表，打印 warning 提示数据已弃用
        let mut rows = conn.query(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name IN ('element_snapshots', 'crawl_checkpoints', 'response_cache')",
            (),
        )
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("old schema query: {e}"))))?;
        let has_old_table = if let Some(row) = rows.next().await
            .map_err(|e| WispError::Storage(StorageError::General(format!("old schema fetch: {e}"))))? {
            let val = row.get_value(0)
                .map_err(|e| WispError::Storage(StorageError::General(format!("old schema get_value: {e}"))))?;
            matches!(val, TursoValue::Integer(1))
        } else {
            false
        };
        if has_old_table {
            tracing::warn!("检测到旧 schema (element_snapshots/crawl_checkpoints/response_cache 三表)，与新版单表 kv 结构不兼容。旧数据已弃用，建议删除 db 文件重新开始。");
        }

        conn.execute_batch(super::migrations::SCHEMA_V1)
            .await
            .map_err(|e| WispError::Storage(StorageError::General(format!("SCHEMA_V1: {e}"))))?;
        Ok(())
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, NULL, ?4)",
            turso::params![namespace, key, value.to_vec(), now],
        )
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso set: {e}"))))?;
        Ok(())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        let mut rows = conn.query(
            "SELECT value FROM kv \
             WHERE namespace = ?1 AND key = ?2 \
               AND (ttl_secs IS NULL OR cached_at + ttl_secs >= CAST(strftime('%s','now') AS INTEGER))",
            turso::params![namespace, key],
        )
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso get: {e}"))))?;
        if let Some(row) = rows.next().await
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso get next: {e}"))))? {
            let val = row.get_value(0)
                .map_err(|e| WispError::Storage(StorageError::General(format!("turso get_value: {e}"))))?;
            match val {
                TursoValue::Blob(b) => Ok(Some(b)),
                TursoValue::Null => Ok(None),
                _ => Err(WispError::Storage(StorageError::General("expected blob".into()))),
            }
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        conn.execute(
            "DELETE FROM kv WHERE namespace = ?1 AND key = ?2",
            turso::params![namespace, key],
        )
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso delete: {e}"))))?;
        Ok(())
    }

    async fn set_with_ttl(
        &self,
        namespace: &str,
        key: &str,
        value: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        let conn = self.db.connect()
            .map_err(|e| WispError::Storage(StorageError::General(format!("turso connect: {e}"))))?;
        let now = chrono::Utc::now().timestamp();
        let ttl_secs = ttl.map(|d| d.as_secs() as i64);
        conn.execute(
            "INSERT OR REPLACE INTO kv (namespace, key, value, ttl_secs, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            turso::params![namespace, key, value.to_vec(), ttl_secs, now],
        )
        .await
        .map_err(|e| WispError::Storage(StorageError::General(format!("turso set_with_ttl: {e}"))))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn make_store() -> SqliteStore {
        SqliteStore::open_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn checkpoint_roundtrip() {
        let store = make_store().await;
        store.set("checkpoint", "spider1", b"state").await.unwrap();
        assert_eq!(store.get("checkpoint", "spider1").await.unwrap().unwrap(), b"state");
        store.delete("checkpoint", "spider1").await.unwrap();
        assert!(store.get("checkpoint", "spider1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ttl_expiry() {
        let store = make_store().await;
        store.set_with_ttl("ns", "k", b"v", Some(Duration::from_secs(1))).await.unwrap();
        // 手动改 cached_at 让它过期
        {
            let conn = store.db.connect().unwrap();
            conn.execute(
                "UPDATE kv SET cached_at = cached_at - 100 WHERE namespace='ns' AND key='k'",
                (),
            ).await.unwrap();
        }
        assert!(store.get("ns", "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ttl_none_never_expires() {
        let store = make_store().await;
        store.set_with_ttl("ns", "k", b"forever", None).await.unwrap();
        assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"forever");
    }

    #[tokio::test]
    async fn namespace_isolation() {
        let store = make_store().await;
        store.set("ns1", "key", b"a").await.unwrap();
        store.set("ns2", "key", b"b").await.unwrap();
        assert_eq!(store.get("ns1", "key").await.unwrap().unwrap(), b"a");
        assert_eq!(store.get("ns2", "key").await.unwrap().unwrap(), b"b");
    }

    /// 旧 schema 检测：存在旧三表时不应破坏新 kv 表功能。
    #[tokio::test]
    async fn old_schema_detection_does_not_break_new_store() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_old_schema.db");

        // 第一次打开：创建新 kv schema 并写入数据
        {
            let store = SqliteStore::open(&db_path).await.unwrap();
            store.set("ns", "k", b"v").await.unwrap();
        }

        // 模拟旧 db：直接注入旧三表（用 turso 直接执行）
        {
            let db = Builder::new_local(db_path.to_str().unwrap())
                .build()
                .await
                .unwrap();
            let conn = db.connect().unwrap();
            conn.execute_batch(
                "CREATE TABLE element_snapshots (url TEXT, key TEXT);
                 CREATE TABLE crawl_checkpoints (spider_name TEXT, state BLOB);
                 CREATE TABLE response_cache (url TEXT, method TEXT);",
            ).await.unwrap();
        }

        // 重新打开：应检测到旧 schema（打印 warning），但新 kv 表仍可用
        let store = SqliteStore::open(&db_path).await.unwrap();
        // 旧数据仍可读
        assert_eq!(store.get("ns", "k").await.unwrap().unwrap(), b"v");
        // 新写入仍可工作
        store.set("ns", "k2", b"v2").await.unwrap();
        assert_eq!(store.get("ns", "k2").await.unwrap().unwrap(), b"v2");
    }

    /// 验证 turso 原生 async 不阻塞 runtime：50 次 set 期间后台 task 应继续推进。
    #[tokio::test]
    async fn test_sqlite_store_async_does_not_block_runtime() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{Duration, Instant};

        let store = SqliteStore::open_in_memory().await.expect("open in-memory sqlite");
        let counter = Arc::new(AtomicU32::new(0));
        let c = Arc::clone(&counter);

        let task = tokio::spawn(async move {
            for _ in 0..100 {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        let start = Instant::now();
        for i in 0..50 {
            store
                .set("test_ns", &format!("k{i}"), b"v")
                .await
                .expect("set should succeed");
        }
        let write_elapsed = start.elapsed();

        task.await.unwrap();

        let counter_val = counter.load(Ordering::SeqCst);
        assert!(
            counter_val > 10,
            "后台 task 应在 SQLite 写入期间继续，实际 counter={counter_val}"
        );
        assert!(
            write_elapsed < Duration::from_secs(5),
            "50 次 set 应 < 5s，实际 {write_elapsed:?}"
        );
    }
}
```

- [ ] **Step 3: 编译验证**

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

如果出现 `params!` 宏类型不匹配，调整参数类型（turso 可能要求 `&str` 而非 `String`，或 `Vec<u8>` 而非 `&[u8]`）。

- [ ] **Step 4: 修改调用点 — src/crawl/runner.rs**

```bash
cd /home/weng/wisp && grep -n "SqliteStore::open\|SqliteStore::open_in_memory" src/crawl/runner.rs
```

如果有调用，加 `.await`：

```rust
// 旧
let store = SqliteStore::open(&path)?;

// 新
let store = SqliteStore::open(&path).await?;
```

- [ ] **Step 5: 修改调用点 — src/mcp/**

```bash
cd /home/weng/wisp && grep -rn "SqliteStore::open" src/mcp/ --include="*.rs"
```

逐个加 `.await`。

- [ ] **Step 6: 修改调用点 — tests/**

```bash
cd /home/weng/wisp && grep -rn "SqliteStore::open" tests/ --include="*.rs"
```

逐个加 `.await`。注意测试函数必须是 `async` 且标注 `#[tokio::test]`。

- [ ] **Step 7: 删除 parking_lot::Mutex import（如果不再使用）**

```bash
cd /home/weng/wisp && grep -n "use parking_lot::Mutex" src/storage/sqlite.rs
```

如果 `parking_lot::Mutex` 不再被 sqlite.rs 使用（turso 不需要手动加锁），删除该 import。

- [ ] **Step 8: 运行 sqlite 模块测试**

Run: `cd /home/weng/wisp && cargo test --lib storage::sqlite --features sqlite`
Expected: 全部 sqlite 测试 PASS

- [ ] **Step 9: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 10: Clippy 检查**

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -30`
Expected: 不增加新警告（已有的 601 个警告保持不变）

- [ ] **Step 11: 提交**

```bash
cd /home/weng/wisp
git add Cargo.toml Cargo.lock src/storage/sqlite.rs src/crawl/runner.rs src/mcp/ tests/
git commit -m "perf(storage): turso 替换 rusqlite，原生 async 无需 spawn_blocking"
```

---

## Task 8: Clippy 自动修复 + Code Review

**Files:**
- Modify: 全项目（自动修复 + 手动补充）

**Interfaces:**
- Consumes: `cargo clippy --fix`、`cargo fmt`、`superpowers:requesting-code-review` skill
- Produces: 0 clippy 警告

- [ ] **Step 1: 自动修复**

```bash
cd /home/weng/wisp
cargo clippy --fix --all-targets --all-features --allow-dirty --allow-no-vcs
cargo fmt --all
```

Expected: 自动修复 `uninlined_format_args`、`duration_suboptimal_units`、`unnecessary_literal_bound`、`redundant_clone`、`needless_borrow` 等。

- [ ] **Step 2: 编译验证**

Run: `cd /home/weng/wisp && cargo build --all-features`
Expected: 编译通过

如果自动修复引入编译错误（如 `format!("{url}")` 但 `url` 是 `&Url` 类型），手动修复：

```rust
// 自动修复误改
format!("{url}")  // 编译失败：Url 没有 Display 内联支持

// 手动修复
format!("{}", url)  // 恢复
// 或
format!("{}", url.as_str())  // 显式转 &str
```

- [ ] **Step 3: 全量回归**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 4: 检查剩余 clippy 警告**

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -50`
Expected: 列出剩余无法自动修复的警告

- [ ] **Step 5: 手动修复 type_complexity**

搜索 `type_complexity` 警告，抽取 type alias：

```rust
// 旧
struct MockStore {
    data: Mutex<HashMap<(String, String), (Vec<u8>, Option<Instant>)>>,
}

// 新
type MockStoreData = HashMap<(String, String), (Vec<u8>, Option<Instant>)>;
struct MockStore {
    data: Mutex<MockStoreData>,
}
```

- [ ] **Step 6: 手动修复其他剩余警告**

逐个处理 Step 4 列出的警告。常见的：
- `unnecessary_literal_bound`：trait 方法返回 `&str` 但实际不是字面量 → 保持 `&str` 不变，用 `#[expect(clippy::unnecessary_literal_bound)]` 标注（如果确认是误判）
- `uninlined_format_args` 漏掉的 → 手动改 `format!("{x}")`

- [ ] **Step 7: 验证 0 警告**

Run: `cd /home/weng/wisp && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 0 警告（命令成功退出）

- [ ] **Step 8: Code Review**

调用 `superpowers:requesting-code-review` skill 对整个项目做代码审查，重点：
- 自动修复是否破坏格式字符串语义
- `&'static str` 改造是否真的返回字面量
- type alias 抽取后可读性
- 自动修复引入的任何行为变化

- [ ] **Step 9: 全量回归（最终）**

Run: `cd /home/weng/wisp && cargo test --all-features`
Expected: 全部测试 PASS

- [ ] **Step 10: 集成验证（banzhu-rs）**

```bash
cd /home/weng/banzhu-rs
cargo build
```

Expected: 编译通过（banzhu-rs 依赖 wisp path）

- [ ] **Step 11: 提交**

```bash
cd /home/weng/wisp
git add -A
git commit -m "chore(clippy): 修复全部 clippy 警告 + 自动格式化"
```

---

## 最终验证

- [ ] **F1: 全量测试**

```bash
cd /home/weng/wisp && cargo test --all-features
```

Expected: 全部测试 PASS（约 439 个 + Task 4 新增 1 个 = 440 个）

- [ ] **F2: 0 clippy 警告**

```bash
cd /home/weng/wisp && cargo clippy --all-targets --all-features -- -D warnings
```

Expected: 命令成功退出，0 警告

- [ ] **F3: 集成验证**

```bash
cd /home/weng/banzhu-rs && cargo build
```

Expected: 编译通过

- [ ] **F4: 最终 code review**

用 `superpowers:requesting-code-review` skill 对整个分支做最终审查（MERGE_BASE = 之前 async-concurrency-fix 完成时的 commit）。
