# Task 7 Report: Scheduler 改造为 async + Mutex

## Status: DONE_WITH_CONCERNS

## What was implemented

完全重写 `src/crawl/scheduler.rs`，从同步 struct 改造为 async + `Arc<Mutex<>>` 设计：

- **PrioritizedRequest** (私有) — 保留原 priority + seq 结构，trait impls (PartialEq/Eq/PartialOrd/Ord) 不变
- **SchedulerInner** (私有) — 持有 `heap` / `seen` / `seq`，由 Mutex 守护
- **Scheduler** (公开, `#[derive(Clone)]`) — 包装 `Arc<Mutex<SchedulerInner>>`，可跨 task 共享
- **8 个 async 方法**：`new`, `push`, `pop`, `pending_urls`, `seen_urls`, `len`, `is_empty`, `restore`
- **Clone impl for PrioritizedRequest** — 为 `pending_urls` 的 `.cloned().collect()` 提供
- **fingerprint** helper — `DefaultHasher` 计算 URL 哈希
- 使用 `tokio::sync::Mutex`（async-aware），非 `std::sync::Mutex`

## cargo check 输出摘要

```
error[E0308]: mismatched types
  --> src\crawl\mod.rs:132:19
   |
132 |         while let Some(req) = sched.pop() {
   |                   ^^^^^^^^^   ----------- this expression has type
   |                                `impl futures::Future<Output = std::option::Option<SpiderRequest>>`
   |
   = note: expected opaque type `impl futures::Future<...>`
                      found enum `std::option::Option<_>`
help: consider `await`ing on the Future
   |
132 |         while let Some(req) = sched.pop().await {
```

- **scheduler.rs 错误数：0** ✅
- **mod.rs 错误数：1**（line 132，`sched.pop()` 需要 `.await`）✅ 预期内
- 警告 2 个（与 Task 7 无关：`browser/mod.rs` unused import, `scraper/mod.rs` unused variable）

### 关于"3 个预期错误"的说明

Task brief 提到 `mod.rs` 有 3 处调用点需要修复（line 125, 132, 182）。实际编译时只有 line 132 (`sched.pop()`) 报错，原因是：
- `sched.push(...)` 返回 `Future`，作为语句直接丢弃 Future 不触发编译错误（只是运行时 no-op，Task 8 加 `.await` 即可）
- `sched.pop()` 用于 `while let Some(req) = ...` 模式绑定，类型不匹配触发 E0308

这与 brief 描述的"3 处需要 .await"在语义上一致——只是 Rust 编译器只在类型受约束的位置报错。Task 8 重写 `Engine::run` 时会一并加 `.await`。

## 与 brief 的偏差（已修复）

Brief 中的 verbatim 代码存在 **borrow checker 错误**（E0502），在 `push` 和 `restore` 两处都出现：

```rust
// Brief 原文（编译失败）：
g.heap.push(PrioritizedRequest { req, seq: g.seq });
//                                    ^^^^^^
// immutable borrow of g.seq while g.heap is mutably borrowed
```

**修复**（保持语义不变，2 处同样修改）：
```rust
let seq = g.seq;
g.heap.push(PrioritizedRequest { req, seq });
g.seq += 1;
```

这是 trivial 的 borrow 拆分，把 `g.seq` 先读到局部变量再 push，不改变运行时行为。修改后 scheduler.rs 编译通过。

## Files changed

- `src/crawl/scheduler.rs` — 1 file changed, 91 insertions(+), 13 deletions(-)

## Commits

- `602f76e` — refactor: Scheduler 改造为 async + Mutex 支持并发访问

## Self-review

- [x] 完全替换 scheduler.rs 为 async 版本
- [x] 8 个 async 方法全部存在（new/push/pop/pending_urls/seen_urls/len/is_empty/restore）
- [x] Clone impl for PrioritizedRequest 存在
- [x] fingerprint 函数存在
- [x] `tokio::sync::Mutex` 而非 `std::sync::Mutex`
- [x] 未修改 mod.rs（Task 8 处理）
- [x] 只提交了 scheduler.rs
- [x] scheduler.rs 本身无编译错误
- [x] mod.rs 错误为预期内（deferred to Task 8）

## Concerns

1. **Brief 代码有 borrow checker bug**：verbatim 代码无法编译，已做最小修复（2 处局部变量提取）。Task 8/9 review 时建议同步更新 brief 或注明此偏差。

2. **`seen_urls` placeholder**：返回 `u64` hash 的字符串形式，非真实 URL。这是 brief 明确接受的 stage 1 限制，不在本 task 修复范围。

3. **`push` 调用点静默失败**：`mod.rs:125` 和 `mod.rs:182` 的 `sched.push(...)` 由于未 `.await`，Future 被丢弃，运行时不会真正 push。Task 8 重写 `Engine::run` 时必须一并修复。
