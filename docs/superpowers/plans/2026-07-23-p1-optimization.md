# P1 架构优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 推进 wisp 爬虫框架 4 个低/中风险 P1 优化项，消除每请求全局锁、分离 Scheduler 锁、DRY Method 转换、使 SpiderRequest.meta 跨 checkpoint 持久化。

**Architecture:** (1) `status_codes` / `proxy_clients` 由 `Mutex<HashMap>` 改为 `DashMap`（前者值用 `AtomicUsize`，彻底无锁计数）。(2) Scheduler 将 `seen` 集合（`DashSet`）与 `heap`（独立 `Mutex`）分离，push/pop 不再串行于同一锁。(3) 抽 `Method::as_str()` 替换 3 处重复 match。(4) 为 `SpiderRequest.meta` 加自定义 serde（`Vec<u8>` 承载 JSON 字节），绕过 bincode 不支持 `deserialize_any` 的限制，使 meta 随 checkpoint 往返。

**Tech Stack:** Rust 2021 edition, tokio 异步运行时, dashmap 6（已在 Cargo.toml）, bincode checkpoint, serde_json, wreq HTTP client。

## Global Constraints

- Rust edition 2021，工具链：`cargo build` / `cargo test --lib` / `cargo test --test <name>` 必须通过。
- 所有公开 API（`Spider` trait、`SpiderBuilder`、`Engine::infra/run`、`Method` 枚举、`SpiderRequest` 字段、`Scheduler` 公开方法签名）保持向后兼容，不删除现有方法。
- 禁止切换分支，所有开发在 master 主分支提交。
- 提交信息简短，一行。
- `dashmap` 已是依赖（P0-2 引入），无需新增。
- 遇到危险命令默认执行，不询问。
- 现有测试基线：`cargo test --lib` 206 passed；clippy 28 warnings（基线，不得新增）。

---

## File Structure

- 修改 `src/crawl/observability/stats.rs`：`status_codes` 字段改 `DashMap<u16, AtomicUsize>`，新增 `status_codes_snapshot()` 辅助方法。
- 删除 `src/crawl/stats.rs`：孤立的死文件（mod.rs 无 `mod stats;` 声明，实际用 `observability::stats`），消除混淆。
- 修改 `src/crawl/engine.rs`：`record_status` 改同步无锁；`proxy_clients` 字段及 `fetch_page`/`fetch_page_inner` 签名改 `DashMap`；2 处 snapshot 改用辅助方法；`make_ctx` 测试辅助更新。
- 修改 `src/crawl/runner.rs`：`proxy_clients` 构造改 `DashMap::new()`；1 处 snapshot 改用辅助方法。
- 修改 `src/crawl/scheduling/scheduler.rs`：`Scheduler` 拆 `seen`（`DashSet`）与 `heap`（独立 `Mutex`）。
- 修改 `src/crawl/mod.rs`：`Method` 加 `as_str()`；`SpiderRequest.meta` 加 `#[serde(with)]`。
- 新增 `tests/p1_status_codes_test.rs`、`tests/p1_proxy_clients_test.rs`、`tests/p1_scheduler_test.rs`、`tests/p1_meta_persistence_test.rs`：各 Task 的集成测试。

---

### Task 1: P1-5 Method::as_str() DRY 转换

**Files:**
- Modify: `src/crawl/mod.rs:51-53`
- Modify: `src/crawl/engine.rs:309-314`
- Modify: `src/crawl/middleware/builtin.rs:283-288, 307-312`
- Test: `src/crawl/mod.rs`（追加到现有 `tests` 模块）

**Interfaces:**
- Produces: `pub fn Method::as_str(&self) -> &'static str`，返回 `"GET"/"POST"/"PUT"/"DELETE"`。

- [x] **Step 1: 写失败测试 — Method::as_str 返回标准 HTTP 动词**

在 `src/crawl/mod.rs` 现有 `#[cfg(test)] mod tests` 模块末尾追加（该模块已有其他测试，找到其 `}` 前插入）：

```rust
    fn test_method_as_str_returns_standard_verbs() {
        assert_eq!(Method::Get.as_str(), "GET");
        assert_eq!(Method::Post.as_str(), "POST");
        assert_eq!(Method::Put.as_str(), "PUT");
        assert_eq!(Method::Delete.as_str(), "DELETE");
    }
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --lib crawl::tests::test_method_as_str_returns_standard_verbs`
Expected: 编译失败 — `as_str` 方法不存在（`no method named as_str`）。

- [x] **Step 3: 实现 Method::as_str**

在 `src/crawl/mod.rs:53` 的 `pub enum Method { Get, Post, Put, Delete }` 下方新增 impl 块：

```rust
impl Method {
    /// 返回标准 HTTP 动词字符串（大写）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}
```

- [x] **Step 4: 替换 engine.rs 的 method_str match**

`src/crawl/engine.rs:308-314` 当前：

```rust
    // 提前计算 method_str（缓存查询与写入都需要）
    let method_str = match req.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    };
```

替换为：

```rust
    // 提前计算 method_str（缓存查询与写入都需要）
    let method_str = req.method.as_str();
```

- [x] **Step 5: 替换 builtin.rs 两处 method_str match**

`src/crawl/middleware/builtin.rs:283-288`：

```rust
        let method_str = match req.method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        };
```

替换为：

```rust
        let method_str = req.method.as_str();
```

`src/crawl/middleware/builtin.rs:307-312`：

```rust
            let method_str = match resp.request.method {
                Method::Get => "GET",
                Method::Post => "POST",
                Method::Put => "PUT",
                Method::Delete => "DELETE",
            };
```

替换为：

```rust
            let method_str = resp.request.method.as_str();
```

- [x] **Step 6: 运行测试验证通过**

Run: `cargo test --lib crawl::tests::test_method_as_str_returns_standard_verbs && cargo build`
Expected: 测试 PASS；编译无错（engine.rs/builtin.rs 中 `Method` 仍被引用，无未使用 import 警告）。

- [x] **Step 7: 提交**

```bash
git add src/crawl/mod.rs src/crawl/engine.rs src/crawl/middleware/builtin.rs
git commit -m "refactor: Method::as_str() DRY 3 处字符串转换 (P1-5)"
```

---

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

- [x] **Step 1: 写失败测试 — 并发 record_status 不死锁且计数正确**

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

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_status_codes_test`
Expected: 编译失败 — `record_status` 不可见（pub(crate)），`status_codes_snapshot` 方法不存在。

- [x] **Step 3: 修改 observability/stats.rs — status_codes 字段与 snapshot 方法**

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

- [x] **Step 4: 修改 engine.rs record_status 为同步无锁**

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

- [x] **Step 5: 移除 4 处 record_status 调用的 .await**

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

- [x] **Step 6: 修改 engine.rs snapshot 站点改用辅助方法**

`src/crawl/engine.rs:422` 当前：

```rust
        let status_codes_snapshot = stats.status_codes.lock().await.clone();
```

替换为：

```rust
        let status_codes_snapshot = stats.status_codes_snapshot();
```

- [x] **Step 7: 修改 runner.rs snapshot 站点**

`src/crawl/runner.rs:390` 当前：

```rust
        let status_codes = ctx.state.stats.status_codes.lock().await.clone();
```

替换为：

```rust
        let status_codes = ctx.state.stats.status_codes_snapshot();
```

- [x] **Step 8: 暴露 record_status 供集成测试**

`src/crawl/mod.rs` 的 re-export 区（约 24 行 `pub use observability::stats;` 附近）确认 `SpiderStats` 已通过该 re-export 可见。`record_status` 是 `pub(crate)`，集成测试（外部 crate）无法访问。需在 `src/crawl/engine.rs` 的 `record_status` 定义改为 `pub` 并在 `src/crawl/mod.rs` 追加 re-export：

`src/crawl/engine.rs:553` 的 `pub(crate) fn record_status` → `pub fn record_status`。

`src/crawl/mod.rs` 在 `pub use runner::{Engine, EngineBuilder};` 后追加：

```rust
pub use engine::record_status;
```

- [x] **Step 9: 删除孤立死文件 src/crawl/stats.rs**

`src/crawl/stats.rs` 是死文件（`mod.rs` 无 `mod stats;` 声明，实际 stats 来自 `observability::stats`，见 mod.rs:24 `pub use observability::stats;`）。删除以消除混淆。

Run: `git rm src/crawl/stats.rs`

- [x] **Step 10: 运行测试验证通过**

Run: `cargo test --test p1_status_codes_test && cargo test --lib && cargo build`
Expected: 新测试 PASS；lib 206 测试全绿；编译无错。

- [x] **Step 11: 提交**

```bash
git add src/crawl/observability/stats.rs src/crawl/engine.rs src/crawl/runner.rs src/crawl/mod.rs tests/p1_status_codes_test.rs
git commit -m "perf: status_codes 改用 DashMap 无锁计数 (P1-1a)"
```

注：`git rm src/crawl/stats.rs` 已暂存删除，随本次 commit 一并提交。

---

### Task 3: P1-1b proxy_clients 改用 DashMap

**Files:**
- Modify: `src/crawl/engine.rs:66,638,668,695-705,800`
- Modify: `src/crawl/runner.rs:225`
- Test: `tests/p1_proxy_clients_test.rs`（新建）

**Interfaces:**
- Produces: `EngineShared.proxy_clients: Arc<DashMap<String, Arc<Client>>>`（原 `Arc<Mutex<HashMap<...>>>`）。
- Produces: `fetch_page` / `fetch_page_inner` 参数 `proxy_clients: &DashMap<String, Arc<Client>>`。

- [x] **Step 1: 写失败测试 — 相同 proxy 只构建一次 Client**

新建 `tests/p1_proxy_clients_test.rs`：

```rust
//! P1-1b: proxy_clients 用 DashMap，相同 proxy 复用 Client。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wisp::crawl::engine::fetch_page_inner;
use wisp::crawl::{SpiderRequest, Method};
use wisp::fetcher::FetchMode;
use wisp::http::{Client, Config};

#[tokio::test]
async fn proxy_clients_caches_client_per_proxy_url() {
    // proxy_clients 暴露为 DashMap，验证相同 proxy 两次 fetch 只产生一个缓存条目
    let client = Arc::new(Client::builder().build().unwrap());
    let config = Config::default();
    let proxy_clients = Arc::new(dashmap::DashMap::new());
    let req = SpiderRequest::get("http://127.0.0.1:1/unreachable");

    // 两次 fetch 同一 proxy（连接会失败，但 Client 应被缓存）
    for _ in 0..2 {
        let _ = fetch_page_inner(
            &client,
            &req,
            Some("http://127.0.0.1:1"),
            FetchMode::Http,
            &config,
            &proxy_clients,
        ).await;
    }

    assert_eq!(proxy_clients.len(), 1, "相同 proxy 应只缓存 1 个 Client");
    assert!(proxy_clients.contains_key("http://127.0.0.1:1"));
}
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_proxy_clients_test`
Expected: 编译失败 — `fetch_page_inner` 不可见（`pub(crate)`），`proxy_clients` 类型不匹配（当前是 `Mutex`）。

- [x] **Step 3: 修改 engine.rs — proxy_clients 字段类型**

`src/crawl/engine.rs:66` 当前：

```rust
    pub proxy_clients: Arc<Mutex<HashMap<String, Arc<Client>>>>,
```

替换为：

```rust
    pub proxy_clients: Arc<dashmap::DashMap<String, Arc<Client>>>,
```

- [x] **Step 4: 修改 fetch_page 与 fetch_page_inner 签名**

`src/crawl/engine.rs:631-638` `fetch_page` 签名末参：

```rust
    proxy_clients: &Mutex<HashMap<String, Arc<Client>>>,
```

替换为：

```rust
    proxy_clients: &dashmap::DashMap<String, Arc<Client>>,
```

`src/crawl/engine.rs:668` `fetch_page_inner` 签名同参同样替换。

并将两个函数从 `pub(crate) async fn` 改为 `pub async fn`（供集成测试访问）。即 `src/crawl/engine.rs:631` 的 `pub(crate) async fn fetch_page(` → `pub async fn fetch_page(`，`src/crawl/engine.rs:661` 的 `pub(crate) async fn fetch_page_inner(` → `pub async fn fetch_page_inner(`。

- [x] **Step 5: 修改 fetch_page_inner 内部锁逻辑**

`src/crawl/engine.rs:693-705` 当前：

```rust
    // Http 模式
    // 代理 Client 缓存：相同 proxy URL 复用已建立的连接，避免每请求 TLS 握手
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        let mut cache = proxy_clients.lock().await;
        if !cache.contains_key(proxy) {
            let new_client = Client::builder()
                .timeout(client.config_ref().timeout)
                .proxy(proxy)
                .build()?;
            cache.insert(proxy.to_string(), Arc::new(new_client));
        }
        Some(cache.get(proxy).unwrap().clone())
    } else {
        None
    };
```

替换为（DashMap：快路径 get，慢路径 build 后 entry::or_insert，错误向上传播）：

```rust
    // Http 模式
    // 代理 Client 缓存：相同 proxy URL 复用已建立的连接，避免每请求 TLS 握手
    let proxy_client: Option<Arc<Client>> = if let Some(proxy) = proxy_url {
        if let Some(c) = proxy_clients.get(proxy) {
            Some(c.clone())
        } else {
            // 慢路径：构建新 client（可能失败，错误向上传播）
            let new_client = Client::builder()
                .timeout(client.config_ref().timeout)
                .proxy(proxy)
                .build()?;
            let arc = Arc::new(new_client);
            // 并发安全：若另一 task 已插入，用已存在的；否则用新建的
            Some(proxy_clients.entry(proxy.to_string()).or_insert(arc).clone())
        }
    } else {
        None
    };
```

- [x] **Step 6: 修改 runner.rs 构造**

`src/crawl/runner.rs:225` 当前：

```rust
                proxy_clients: Arc::new(Mutex::new(HashMap::new())),
```

替换为：

```rust
                proxy_clients: Arc::new(dashmap::DashMap::new()),
```

- [x] **Step 7: 修改 engine.rs make_ctx 测试辅助**

`src/crawl/engine.rs:800` 当前：

```rust
                proxy_clients: Arc::new(Mutex::new(HashMap::new())),
```

替换为：

```rust
                proxy_clients: Arc::new(dashmap::DashMap::new()),
```

- [x] **Step 8: 暴露 fetch_page_inner 供集成测试**

`src/crawl/mod.rs` re-export 区追加（紧接 Task 2 的 `pub use engine::record_status;`）：

```rust
pub use engine::{record_status, fetch_page, fetch_page_inner};
```

（若 Task 2 已加 `pub use engine::record_status;`，此处合并为 `pub use engine::{record_status, fetch_page, fetch_page_inner};`。）

- [x] **Step 9: 运行测试验证通过**

Run: `cargo test --test p1_proxy_clients_test && cargo test --lib && cargo build`
Expected: 新测试 PASS；lib 206 测试全绿；编译无错。

- [x] **Step 10: 提交**

```bash
git add src/crawl/engine.rs src/crawl/runner.rs src/crawl/mod.rs tests/p1_proxy_clients_test.rs
git commit -m "perf: proxy_clients 改用 DashMap 消除全局锁 (P1-1b)"
```

---

### Task 4: P1-2 Scheduler seen/heap 锁分离

**Files:**
- Modify: `src/crawl/scheduling/scheduler.rs:1-189`
- Test: `tests/p1_scheduler_test.rs`（新建）

**Interfaces:**
- Consumes: `dashmap::DashSet`（dashmap crate 提供，无需新增依赖）。
- Produces: `Scheduler` 内部结构拆为 `heap: Arc<Mutex<HeapInner>>` + `seen_exact: Arc<DashSet<String>>` + `seen_fp: Arc<DashSet<u64>>`；公开方法签名（`push`/`pop`/`pending_urls`/`seen_urls`/`len`/`is_empty`/`restore`）不变。

- [x] **Step 1: 写失败测试 — 并发 push/pop 不死锁且去重正确**

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

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_scheduler_test`
Expected: 并发测试可能 PASS（原 Mutex 也不死锁，只是串行）或 PASS 但慢。关键看后续 Step 重构后仍 PASS。先记录基线时间。

Run: `cargo test --test p1_scheduler_test -- --nocapture 2>&1 | grep "test result"`
Expected: 2 passed（基线）。

- [x] **Step 3: 重构 scheduler.rs — 拆分 seen 与 heap**

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

- [x] **Step 4: 重构 with_strategy 构造**

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

- [x] **Step 5: 重构 push — seen 先查再锁 heap**

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

- [x] **Step 6: 重构 pop**

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

- [x] **Step 7: 重构 pending_urls**

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

- [x] **Step 8: 重构 seen_urls**

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

- [x] **Step 9: 重构 len / is_empty**

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

- [x] **Step 10: 重构 restore**

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

- [x] **Step 11: 运行测试验证通过**

Run: `cargo test --test p1_scheduler_test && cargo test --lib crawl::scheduling && cargo test --lib`
Expected: 新 2 测试 PASS；scheduler 现有单元测试全绿；lib 206 全绿。

- [x] **Step 12: 提交**

```bash
git add src/crawl/scheduling/scheduler.rs tests/p1_scheduler_test.rs
git commit -m "perf: Scheduler seen/heap 锁分离 (P1-2)"
```

---

### Task 5: P1-7 SpiderRequest.meta 跨 checkpoint 持久化

**Files:**
- Modify: `src/crawl/mod.rs:69-95`
- Test: `tests/p1_meta_persistence_test.rs`（新建）

**Interfaces:**
- Produces: `SpiderRequest.meta` 由 `#[serde(skip)]` 改为 `#[serde(with = "meta_serde")]`，使 meta 随 bincode checkpoint 序列化往返。
- Produces: 私有 `meta_serde` 模块（`serialize`/`deserialize` 两个函数，把 `serde_json::Value` 编码为 `Vec<u8>` JSON 字节供 bincode 处理）。

- [x] **Step 1: 写失败测试 — meta 经 bincode 往返保持一致**

新建 `tests/p1_meta_persistence_test.rs`：

```rust
//! P1-7: SpiderRequest.meta 随 bincode checkpoint 持久化。

use wisp::crawl::SpiderRequest;
use serde_json::json;

#[test]
fn meta_survives_bincode_roundtrip() {
    let req = SpiderRequest::get("https://example.com/page")
        .with_meta(json!({
            "source_page": "https://example.com/list",
            "page_index": 42,
            "tags": ["a", "b"],
            "nested": { "x": 1.5, "y": null }
        }));

    let bytes = bincode::serialize(&req).expect("serialize");
    let restored: SpiderRequest = bincode::deserialize(&bytes).expect("deserialize");

    assert_eq!(restored.url, "https://example.com/page");
    assert_eq!(restored.meta, req.meta, "meta 必须往返保持一致");
    // 抽查嵌套字段
    assert_eq!(restored.meta["page_index"], 42);
    assert_eq!(restored.meta["tags"][1], "b");
    assert_eq!(restored.meta["nested"]["y"], serde_json::Value::Null);
}

#[test]
fn meta_default_null_when_absent() {
    let req = SpiderRequest::get("https://example.com/x");
    let bytes = bincode::serialize(&req).expect("serialize");
    let restored: SpiderRequest = bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(restored.meta, serde_json::Value::Null);
}
```

- [x] **Step 2: 运行测试验证失败**

Run: `cargo test --test p1_meta_persistence_test`
Expected: `meta_survives_bincode_roundtrip` 失败 — `restored.meta` 为 `Value::Null`（因 `#[serde(skip)]` 跳过序列化），不等于原 meta。`meta_default_null_when_absent` 可能 PASS（恰好 Value::Null）。

- [x] **Step 3: 在 mod.rs 添加 meta_serde 模块**

`src/crawl/mod.rs` 在 `pub enum Method` 定义之前（约 line 51 前）插入私有 serde 辅助模块：

```rust
/// 自定义 serde：把 `serde_json::Value` 编码为 `Vec<u8>` JSON 字节，
/// 绕过 bincode 1.x 不支持 `deserialize_any` 的限制，使 meta 随 checkpoint 往返。
mod meta_serde {
    use serde::{Deserializer, Serialize, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(v: &Value, s: S) -> Result<S::Ok, S::Error> {
        let bytes = serde_json::to_vec(v).map_err(serde::ser::Error::custom)?;
        bytes.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        let bytes = Vec::<u8>::deserialize(d)?;
        serde_json::from_slice(&bytes).map_err(serde::de::Error::custom)
    }
}
```

- [x] **Step 4: 修改 SpiderRequest.meta 的 serde 属性**

`src/crawl/mod.rs:75-83` 当前：

```rust
    // Task 3：必须用 `#[serde(skip)]` 而非 `#[serde(default)]`。
    // `serde_json::Value` 的 Deserialize 依赖 `deserialize_any`，bincode 1.x 不支持；
    // 用 `#[serde(default)]` 会让 `bincode::deserialize::<CrawlState>`（含 SpiderRequest）
    // 在 checkpoint 恢复路径抛 `DeserializeAnyNotSupported`，导致 seen/pending 全部丢失。
    // `#[serde(skip)]` 在序列化与反序列化两端都跳过 meta（用 Value::Null 默认值），
    // 与 Task 9 的既定行为一致（meta 当前不从 checkpoint 读回）。
    // 83cb940 误改为 `#[serde(default)]` 引入回归，此处恢复。
    #[serde(skip)]
    pub meta: Value,
```

替换为（保留约束说明，更新为 with 方案）：

```rust
    // P1-7：用 `#[serde(with = "meta_serde")]` 使 meta 随 bincode checkpoint 往返。
    // bincode 1.x 不支持 `serde_json::Value` 的 `deserialize_any`，
    // 故通过 `meta_serde` 把 Value 编码为 `Vec<u8>` JSON 字节，bincode 可处理。
    #[serde(with = "meta_serde")]
    pub meta: Value,
```

- [x] **Step 5: 运行测试验证通过**

Run: `cargo test --test p1_meta_persistence_test && cargo test --lib`
Expected: 2 测试 PASS；lib 206 全绿（含 checkpoint 相关测试 save_checkpoint_persists_seen_urls 等）。

- [x] **Step 6: 运行 checkpoint 集成测试验证不回归**

Run: `cargo test --test cr_fix_engine_test --test engine_infra_test`
Expected: 全绿（checkpoint 恢复路径未因 meta 序列化改变而破坏）。

- [x] **Step 7: 提交**

```bash
git add src/crawl/mod.rs tests/p1_meta_persistence_test.rs
git commit -m "feat: SpiderRequest.meta 跨 checkpoint 持久化 (P1-7)"
```

---

### Task 6: 最终回归验证与清理

**Files:**
- 全量测试
- `docs/superpowers/plans/2026-07-23-p1-optimization.md`（本文件，标记完成）

**Interfaces:**
- 无新增接口

- [x] **Step 1: 全量 lib + 集成测试**

Run: `cargo test --lib`
Expected: 206+ 测试全绿（可能因新增单元测试略增）。

Run: `cargo test --test p1_status_codes_test --test p1_proxy_clients_test --test p1_scheduler_test --test p1_meta_persistence_test --test p0_autoscale_test --test p0_dashmap_test --test engine_infra_test --test crawl_concurrency_test --test multi_spider_test --test builder_api_test --test cr_fix_engine_test`
Expected: 全部 PASS。

- [x] **Step 2: 验证 clippy 无新警告**

Run: `cargo clippy --lib 2>&1 | grep "generated.*warnings"`
Expected: `28 warnings`（与 P0 完成后基线一致，不新增）。

- [x] **Step 3: 标记 plan 完成**

在 plan 文件中将所有 `- [ ]` 改为 `- [x]`：

Run: `sed -i 's/^- \[ \]/- [x]/g' docs/superpowers/plans/2026-07-23-p1-optimization.md`

- [x] **Step 4: 提交 plan 完成标记**

`docs/` 已被本地 `.gitignore`（工作区修改），plan 文件为本地工作文件，无法 commit。此 Step 为 no-op，跳过提交，仅本地标记完成。

---

## Self-Review

### 1. Spec coverage

| Spec 项 | Plan Task |
|---|---|
| P1-1 status_codes/proxy_clients 每请求锁 | Task 2 (status_codes DashMap) + Task 3 (proxy_clients DashMap) |
| P1-2 Scheduler 单 Mutex<BinaryHeap> | Task 4 (seen DashSet + heap 独立 Mutex) |
| P1-5 Method 枚举与转换重复 | Task 1 (Method::as_str + 替换 3 处) |
| P1-7 SpiderRequest.meta 不持久化 | Task 5 (meta_serde 自定义 serde) |
| P1-3 反检测能力薄弱 | 不在本计划（大特性，单独 spec） |
| P1-4 Storage 后端单一 | 不在本计划（大特性，单独 spec） |
| P1-6 双轨重试逻辑 | 不在本计划（核心抓取改动大、回归风险高） |

覆盖 4/7 P1 项，其余 3 项按风险/规模显式排除，已在 plan 头部说明。

### 2. Placeholder scan

无 TBD/TODO/"实现细节后补"。每步含完整代码或精确命令。

### 3. Type consistency

- `record_status`: Task 2 定义为 `pub fn record_status(stats: &Arc<SpiderStats>, status: u16)`（同步），Step 5 移除调用点 `.await`。Task 2 Step 8 re-export `pub use engine::record_status;`。一致。
- `fetch_page_inner`: Task 3 定义为 `pub async fn fetch_page_inner(..., proxy_clients: &dashmap::DashMap<String, Arc<Client>>)`, Step 8 re-export。一致。
- `status_codes_snapshot`: Task 2 Step 3 定义为 `pub fn status_codes_snapshot(&self) -> HashMap<u16, usize>`，Task 2 Step 6/7 调用。一致。
- `Scheduler`: Task 4 拆分后字段 `heap`/`seen_exact`/`seen_fp`/`strategy`，公开方法签名不变。一致。
- `meta_serde`: Task 5 Step 3 定义 `serialize`/`deserialize`，Step 4 `#[serde(with = "meta_serde")]` 引用。一致。

### 执行顺序

Task 1（最小、无依赖）→ Task 2（status_codes）→ Task 3（proxy_clients，依赖 Task 2 的 re-export 位置合并）→ Task 4（Scheduler，独立）→ Task 5（meta，独立）→ Task 6（回归）。

Task 3 Step 8 与 Task 2 Step 8 都修改 `src/crawl/mod.rs` 的 re-export 区，按顺序执行时 Task 3 合并为 `pub use engine::{record_status, fetch_page, fetch_page_inner};`（覆盖 Task 2 的单行）。
