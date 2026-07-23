### Task 4: P1-2 Scheduler seen/heap 锁分离

**Files:**
- Modify: `src/crawl/scheduling/scheduler.rs:1-189`
- Test: `tests/p1_scheduler_test.rs`（新建）

**Interfaces:**
- Consumes: `dashmap::DashSet`（dashmap crate 提供，无需新增依赖）。
- Produces: `Scheduler` 内部结构拆为 `heap: Arc<Mutex<HeapInner>>` + `seen_exact: Arc<DashSet<String>>` + `seen_fp: Arc<DashSet<u64>>`；公开方法签名（`push`/`pop`/`pending_urls`/`seen_urls`/`len`/`is_empty`/`restore`）不变。

- [ ] **Step 1: 写失败测试 — 并发 push/pop 不死锁且去重正确**

新建 `tests/p1_scheduler_test.rs`：

```rust
//! P1-2: Scheduler seen/heap 分离，并发不死锁。

use wisp::crawl::scheduler::{Scheduler, DedupStrategy};
use wisp::crawl::SpiderRequest;

#[tokio::test]
async fn scheduler_concurrent_push_pop_dedup_correct() {
    let sched = Scheduler::new();
    // 并发 push 1000 个 URL（含 50% 重复），再 pop 全部
    let pushers: Vec<_> = (0..10)
        .map(|tid| {
            let s = sched.clone();
            tokio::spawn(async move {
                for i in 0..100 {
                    // tid*100+i，偶数为重复（0,2,4.. 跨线程共享同一组 URL）
                    let url = format!("https://example.com/{}", if tid % 2 == 0 { i } else { 1000 + tid * 100 + i });
                    s.push(SpiderRequest::get(&url)).await;
                }
            })
        })
        .collect();
    for h in pushers { h.await.unwrap(); }

    // pop 全部，验证无 panic、数量 = 唯一 URL 数
    let mut popped = 0;
    while sched.pop().await.is_some() {
        popped += 1;
    }
    // 5 个偶数 tid 各推 0..99（100 个，但跨偶数 tid 重复同一组 0..99）→ 去重后 100 个
    // 5 个奇数 tid 各推 1000+tid*100+i（500 个唯一）→ 500 个
    // 总计 600 个唯一
    assert_eq!(popped, 600, "去重后应剩 600 个唯一 URL");
}

#[tokio::test]
async fn scheduler_fingerprint_strategy_seen_split_works() {
    let sched = Scheduler::with_strategy(DedupStrategy::Fingerprint);
    sched.push(SpiderRequest::get("https://example.com/a")).await;
    // 重复 push 同 URL 应被去重
    sched.push(SpiderRequest::get("https://example.com/a")).await;
    assert_eq!(sched.len().await, 1);
    let seen = sched.seen_urls().await;
    assert_eq!(seen.len(), 1, "Fingerprint 模式 seen 应含 1 个 hash");
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_scheduler_test`
Expected: 并发测试可能 PASS（原 Mutex 也不死锁，只是串行）或 PASS 但慢。关键看后续 Step 重构后仍 PASS。先记录基线时间。

Run: `cargo test --test p1_scheduler_test -- --nocapture 2>&1 | grep "test result"`
Expected: 2 passed（基线）。

- [ ] **Step 3: 重构 scheduler.rs — 拆分 seen 与 heap**

`src/crawl/scheduling/scheduler.rs` 顶部 imports（line 8-14）当前：

```rust
use crate::crawl::SpiderRequest;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BinaryHeap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::Mutex;
```

替换为（新增 DashSet）：

```rust
use crate::crawl::SpiderRequest;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BinaryHeap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use dashmap::DashSet;
use tokio::sync::Mutex;
```

`SchedulerInner` 与 `Scheduler` 定义（line 50-65）当前：

```rust
struct SchedulerInner {
    heap: BinaryHeap<PrioritizedRequest>,
    seen_exact: HashSet<String>,
    seen_fp: HashSet<u64>,
    strategy: DedupStrategy,
    seq: u64,
}

#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<Mutex<SchedulerInner>>,
}
```

替换为（seen 用 DashSet 独立，heap + seq 共享一个 Mutex，strategy 是 Copy 存外部）：

```rust
/// heap 与 seq 共享一个 Mutex（push/pop 需要原子读 seq + push/pop）。
struct HeapInner {
    heap: BinaryHeap<PrioritizedRequest>,
    seq: u64,
}

/// Scheduler：seen 集合（DashSet，无锁）与 heap（独立 Mutex）分离。
///
/// push 时先查/插 seen（DashSet，无锁），命中才锁 heap 入队；
/// pop 时只锁 heap。两者不再串行于同一锁。
#[derive(Clone)]
pub struct Scheduler {
    heap: Arc<Mutex<HeapInner>>,
    seen_exact: Arc<DashSet<String>>,
    seen_fp: Arc<DashSet<u64>>,
    strategy: DedupStrategy,
}
```

- [ ] **Step 4: 重构 with_strategy 构造**

`src/crawl/scheduling/scheduler.rs:73-83` 当前：

```rust
    pub fn with_strategy(strategy: DedupStrategy) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SchedulerInner {
                heap: BinaryHeap::new(),
                seen_exact: HashSet::new(),
                seen_fp: HashSet::new(),
                strategy,
                seq: 0,
            })),
        }
    }
```

替换为：

```rust
    pub fn with_strategy(strategy: DedupStrategy) -> Self {
        Self {
            heap: Arc::new(Mutex::new(HeapInner { heap: BinaryHeap::new(), seq: 0 })),
            seen_exact: Arc::new(DashSet::new()),
            seen_fp: Arc::new(DashSet::new()),
            strategy,
        }
    }
```

- [ ] **Step 5: 重构 push — seen 先查再锁 heap**

`src/crawl/scheduling/scheduler.rs:86-97` 当前：

```rust
    pub async fn push(&self, req: SpiderRequest) {
        let mut g = self.inner.lock().await;
        let is_new = match g.strategy {
            DedupStrategy::Exact => g.seen_exact.insert(req.url.clone()),
            DedupStrategy::Fingerprint => g.seen_fp.insert(fingerprint(&req.url)),
        };
        if is_new {
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seq += 1;
        }
    }
```

替换为（先 DashSet 去重，命中才锁 heap）：

```rust
    pub async fn push(&self, req: SpiderRequest) {
        // seen 去重（DashSet 无锁，不阻塞 pop）
        let is_new = match self.strategy {
            DedupStrategy::Exact => self.seen_exact.insert(req.url.clone()),
            DedupStrategy::Fingerprint => self.seen_fp.insert(fingerprint(&req.url)),
        };
        if is_new {
            let mut g = self.heap.lock().await;
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seq += 1;
        }
    }
```

- [ ] **Step 6: 重构 pop**

`src/crawl/scheduling/scheduler.rs:100-103` 当前：

```rust
    pub async fn pop(&self) -> Option<SpiderRequest> {
        let mut g = self.inner.lock().await;
        g.heap.pop().map(|p| p.req)
    }
```

替换为：

```rust
    pub async fn pop(&self) -> Option<SpiderRequest> {
        let mut g = self.heap.lock().await;
        g.heap.pop().map(|p| p.req)
    }
```

- [ ] **Step 7: 重构 pending_urls**

`src/crawl/scheduling/scheduler.rs:106-114` 当前：

```rust
    pub async fn pending_urls(&self) -> Vec<SpiderRequest> {
        let g = self.inner.lock().await;
        let mut reqs: Vec<PrioritizedRequest> = g.heap.iter().cloned().collect();
        reqs.sort_by(|a, b| b.cmp(a));
        reqs.into_iter().map(|p| p.req).collect()
    }
```

替换为：

```rust
    pub async fn pending_urls(&self) -> Vec<SpiderRequest> {
        let g = self.heap.lock().await;
        let mut reqs: Vec<PrioritizedRequest> = g.heap.iter().cloned().collect();
        reqs.sort_by(|a, b| b.cmp(a));
        reqs.into_iter().map(|p| p.req).collect()
    }
```

- [ ] **Step 8: 重构 seen_urls**

`src/crawl/scheduling/scheduler.rs:119-125` 当前：

```rust
    pub async fn seen_urls(&self) -> HashSet<String> {
        let g = self.inner.lock().await;
        match g.strategy {
            DedupStrategy::Exact => g.seen_exact.clone(),
            DedupStrategy::Fingerprint => g.seen_fp.iter().map(|h| h.to_string()).collect(),
        }
    }
```

替换为（DashSet 快照不阻塞 heap）：

```rust
    pub async fn seen_urls(&self) -> HashSet<String> {
        match self.strategy {
            DedupStrategy::Exact => self.seen_exact.iter().map(|s| s.clone()).collect(),
            DedupStrategy::Fingerprint => self.seen_fp.iter().map(|h| h.to_string()).collect(),
        }
    }
```

- [ ] **Step 9: 重构 len / is_empty**

`src/crawl/scheduling/scheduler.rs:128-134` 当前：

```rust
    pub async fn len(&self) -> usize {
        self.inner.lock().await.heap.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.heap.is_empty()
    }
```

替换为：

```rust
    pub async fn len(&self) -> usize {
        self.heap.lock().await.heap.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.heap.lock().await.heap.is_empty()
    }
```

- [ ] **Step 10: 重构 restore**

`src/crawl/scheduling/scheduler.rs:137-172` 当前 `restore` 整段引用 `g.strategy`、`g.heap`、`g.seen_exact`、`g.seen_fp`、`g.seq`。替换为（清 seen DashSet + 清 heap Mutex + 重建）：

```rust
    /// Replace inner state (for checkpoint restore).
    pub async fn restore(&self, pending: Vec<SpiderRequest>, seen: HashSet<String>) {
        // 清 seen（DashSet）
        self.seen_exact.clear();
        self.seen_fp.clear();
        // 清 heap + seq（Mutex）
        {
            let mut g = self.heap.lock().await;
            g.heap.clear();
            g.seq = 0;
        }
        // Rebuild seen set
        for url in &seen {
            match self.strategy {
                DedupStrategy::Exact => {
                    self.seen_exact.insert(url.clone());
                }
                DedupStrategy::Fingerprint => {
                    // seen_urls() 在 Fingerprint 模式下返回 u64 哈希的十进制字符串，
                    // 直接 parse 回 u64 即可，不能再 fingerprint（会产生不同 u64）。
                    if let Ok(h) = url.parse::<u64>() {
                        self.seen_fp.insert(h);
                    }
                }
            }
        }
        // Re-queue pending (force insert even if in seen set)
        let mut g = self.heap.lock().await;
        for req in pending {
            match self.strategy {
                DedupStrategy::Exact => {
                    self.seen_exact.insert(req.url.clone());
                }
                DedupStrategy::Fingerprint => {
                    self.seen_fp.insert(fingerprint(&req.url));
                }
            }
            let seq = g.seq;
            g.heap.push(PrioritizedRequest { req, seq });
            g.seq += 1;
        }
    }
```

- [ ] **Step 11: 运行测试验证通过**

Run: `cargo test --test p1_scheduler_test && cargo test --lib crawl::scheduling && cargo test --lib`
Expected: 新 2 测试 PASS；scheduler 现有单元测试全绿；lib 206 全绿。

- [ ] **Step 12: 提交**

```bash
git add src/crawl/scheduling/scheduler.rs tests/p1_scheduler_test.rs
git commit -m "perf: Scheduler seen/heap 锁分离 (P1-2)"
```

---

