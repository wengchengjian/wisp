### Task 2: P1-1a status_codes 改用 DashMap 无锁计数

**Files:**
- Modify: `src/crawl/observability/stats.rs`
- Modify: `src/crawl/engine.rs:166,185,301,450,422,553-556`
- Modify: `src/crawl/runner.rs:390`
- Delete: `src/crawl/stats.rs`（孤立死文件）
- Test: `tests/p1_status_codes_test.rs`（新建）

**Interfaces:**
- Produces: `pub fn SpiderStats::status_codes_snapshot(&self) -> HashMap<u16, usize>` — 无锁快照计数。
- Produces: `fn record_status(stats: &Arc<SpiderStats>, status: u16)` — 改为同步函数（不再 async），内部用 DashMap entry 原子累加。

- [ ] **Step 1: 写失败测试 — 并发 record_status 不死锁且计数正确**

新建 `tests/p1_status_codes_test.rs`：

```rust
//! P1-1a: status_codes 用 DashMap<u16, AtomicUsize> 无锁计数。

use std::sync::Arc;
use wisp::crawl::SpiderStats;

#[tokio::test]
async fn status_codes_concurrent_increment_is_correct() {
    let stats = Arc::new(SpiderStats::new());
    // 并发对同一状态码累加，验证无死锁且计数正确
    let handles: Vec<_> = (0..50)
        .map(|_| {
            let s = stats.clone();
            tokio::spawn(async move {
                for _ in 0..100 {
                    wisp::crawl::record_status(&s, 200);
                    wisp::crawl::record_status(&s, 404);
                }
            })
        })
        .collect();
    for h in handles { h.await.unwrap(); }

    let snap = stats.status_codes_snapshot();
    assert_eq!(snap.get(&200).copied(), Some(5000), "200 计数应为 50*100");
    assert_eq!(snap.get(&404).copied(), Some(5000), "404 计数应为 50*100");
    assert_eq!(snap.len(), 2, "仅 2 个状态码");
}

#[tokio::test]
async fn status_codes_snapshot_returns_empty_for_fresh_stats() {
    let stats = SpiderStats::new();
    assert!(stats.status_codes_snapshot().is_empty());
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_status_codes_test`
Expected: 编译失败 — `record_status` 不可见（pub(crate)），`status_codes_snapshot` 方法不存在。

- [ ] **Step 3: 修改 observability/stats.rs — status_codes 字段与 snapshot 方法**

`src/crawl/observability/stats.rs` 当前：

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
```

替换 imports 为：

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use dashmap::DashMap;
```

字段（line 20）：

```rust
    pub status_codes: Mutex<HashMap<u16, usize>>,
```

替换为：

```rust
    pub status_codes: DashMap<u16, AtomicUsize>,
```

构造（line 35）：

```rust
            status_codes: Mutex::new(HashMap::new()),
```

替换为：

```rust
            status_codes: DashMap::new(),
```

在 `impl SpiderStats` 的 `elapsed` 方法后追加 `status_codes_snapshot`：

```rust
    /// 无锁快照状态码计数为 HashMap<u16, usize>。
    pub fn status_codes_snapshot(&self) -> HashMap<u16, usize> {
        self.status_codes
            .iter()
            .map(|r| (*r.key(), r.value().load(Ordering::SeqCst)))
            .collect()
    }
```

- [ ] **Step 4: 修改 engine.rs record_status 为同步无锁**

`src/crawl/engine.rs:553-556` 当前：

```rust
async fn record_status(stats: &Arc<SpiderStats>, status: u16) {
    let mut m = stats.status_codes.lock().await;
    *m.entry(status).or_insert(0) += 1;
}
```

替换为：

```rust
/// 同步记录状态码计数（DashMap entry 原子累加，无 await）。
pub(crate) fn record_status(stats: &Arc<SpiderStats>, status: u16) {
    stats
        .status_codes
        .entry(status)
        .and_modify(|c| { c.fetch_add(1, Ordering::Relaxed); })
        .or_insert(AtomicUsize::new(1));
}
```

- [ ] **Step 5: 移除 4 处 record_status 调用的 .await**

`src/crawl/engine.rs` 共 4 处调用（行号约 166、185、301、450），形如：

```rust
            record_status(stats, resp.status).await;
```

替换为（去掉 `.await`）：

```rust
            record_status(stats, resp.status);
```

用以下命令定位全部 4 处后逐个编辑（每处上下文不同，需单独 Edit）：

Run: `grep -n "record_status.*\.await" src/crawl/engine.rs`
Expected: 4 行匹配。

- [ ] **Step 6: 修改 engine.rs snapshot 站点改用辅助方法**

`src/crawl/engine.rs:422` 当前：

```rust
        let status_codes_snapshot = stats.status_codes.lock().await.clone();
```

替换为：

```rust
        let status_codes_snapshot = stats.status_codes_snapshot();
```

- [ ] **Step 7: 修改 runner.rs snapshot 站点**

`src/crawl/runner.rs:390` 当前：

```rust
        let status_codes = ctx.state.stats.status_codes.lock().await.clone();
```

替换为：

```rust
        let status_codes = ctx.state.stats.status_codes_snapshot();
```

- [ ] **Step 8: 暴露 record_status 供集成测试**

`src/crawl/mod.rs` 的 re-export 区（约 24 行 `pub use observability::stats;` 附近）确认 `SpiderStats` 已通过该 re-export 可见。`record_status` 是 `pub(crate)`，集成测试（外部 crate）无法访问。需在 `src/crawl/engine.rs` 的 `record_status` 定义改为 `pub` 并在 `src/crawl/mod.rs` 追加 re-export：

`src/crawl/engine.rs:553` 的 `pub(crate) fn record_status` → `pub fn record_status`。

`src/crawl/mod.rs` 在 `pub use runner::{Engine, EngineBuilder};` 后追加：

```rust
pub use engine::record_status;
```

- [ ] **Step 9: 删除孤立死文件 src/crawl/stats.rs**

`src/crawl/stats.rs` 是死文件（`mod.rs` 无 `mod stats;` 声明，实际 stats 来自 `observability::stats`，见 mod.rs:24 `pub use observability::stats;`）。删除以消除混淆。

Run: `git rm src/crawl/stats.rs`

- [ ] **Step 10: 运行测试验证通过**

Run: `cargo test --test p1_status_codes_test && cargo test --lib && cargo build`
Expected: 新测试 PASS；lib 206 测试全绿；编译无错。

- [ ] **Step 11: 提交**

```bash
git add src/crawl/observability/stats.rs src/crawl/engine.rs src/crawl/runner.rs src/crawl/mod.rs tests/p1_status_codes_test.rs
git commit -m "perf: status_codes 改用 DashMap 无锁计数 (P1-1a)"
```

注：`git rm src/crawl/stats.rs` 已暂存删除，随本次 commit 一并提交。

---

