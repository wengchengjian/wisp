# Task 7 Review: Scheduler 改造为 async + Mutex

## Spec Compliance

- ✅ **Spec compliant** — 全部要求已实现，与 brief verbatim 代码一致（仅 2 处必要的 borrow 修复）。

逐项验证：

| Brief 要求 | 实现位置 | 状态 |
|---|---|---|
| 完全替换 `src/crawl/scheduler.rs` | 全文重写，91+ / 13- | ✅ |
| `Scheduler: #[derive(Clone)]` | scheduler.rs:41 | ✅ |
| `Scheduler` 包装 `Arc<Mutex<SchedulerInner>>` | scheduler.rs:43 | ✅ |
| `SchedulerInner` 私有 + 3 字段 | scheduler.rs:34-38 | ✅ |
| 8 个 async 方法 | scheduler.rs:47/58/69/75/86/97/101/106 | ✅ |
| `PrioritizedRequest` 私有 + 手动 Clone | scheduler.rs:14-17, 128-132 | ✅ |
| `fingerprint(url: &str) -> u64` | scheduler.rs:134-138 | ✅ |
| `push` 用 `seen.insert(fp)` 去重 | scheduler.rs:61 | ✅ |
| `pop` 返回最高优先级 | scheduler.rs:69-72 | ✅ |
| `restore` 重建 heap + seen | scheduler.rs:106-124 | ✅ |
| 未修改 `src/crawl/mod.rs` | `git diff --name-only` 仅 scheduler.rs | ✅ |
| 仅提交 scheduler.rs | `git show --stat 602f76e` 仅 1 文件 | ✅ |

补充验证（cargo check）：
- scheduler.rs 编译错误数：**0** ✅
- mod.rs 编译错误数：**1**（line 132 `sched.pop()` 需要 `.await`）✅ 预期内
- 其余 2 个 warning 与本 task 无关（browser/mod.rs、scraper/mod.rs）

## Strengths

1. **零偏差实现 brief**：除 borrow 修复外，结构体定义、trait impls、方法签名、字段类型、注释文案都与 brief verbatim 代码一致，便于后续 review 与 Task 8/9 对接。
2. **Borrow 修复是最小且语义保持的**：
   - `push` (scheduler.rs:62-64)：`let seq = g.seq;` → `g.heap.push(PrioritizedRequest { req, seq });` → `g.seq += 1;`
   - `restore` (scheduler.rs:119-122)：同样模式
   - 仅多出 `let seq = g.seq;` 一行局部变量绑定，运行时行为与 brief 原意图完全一致（push 时取当前 seq，push 后递增）。
3. **正确的 async 选择**：使用 `tokio::sync::Mutex`（async-aware）而非 `std::sync::Mutex`，避免在 `.await` 持锁导致死锁/阻塞 runtime 的常见陷阱。
4. **接口语义正确**：
   - `push` 通过 `g.seen.insert(fp)` 的返回值（true 表示新插入）决定是否入堆，去重语义符合 brief。
   - `pop` 通过 `BinaryHeap::pop` 自然返回 max-heap 顶（最高优先级）。
   - `pending_urls` 使用 `b.cmp(a)` 排序 = 降序 = 最高优先级在前，与 `pop` 顺序一致。
   - `restore` 注释明确"Force insert even if seen"，且代码确实无条件 `g.heap.push`（不依赖 `seen.insert` 的返回值），符合 brief 要求。
5. **Edge cases 处理正确**：
   - 空 heap：`pop` 返回 `None`，`pending_urls` 返回空 `Vec`，`is_empty` 返回 `true`。
   - 重复 URL：`push` 第二次同 URL 不会入堆。
   - `restore` 空输入：`clear` + `seq = 0`，两个 for 循环均不执行。
6. **`SpiderRequest` 已 `#[derive(Clone)]`**（crawl/mod.rs:22），手动 `Clone for PrioritizedRequest` 调用 `self.req.clone()` 合法可用。

## Issues

### Critical (Must Fix)
无。

### Important (Should Fix)
无。

### Minor (Nice to Have)

1. **`new()` 不是 async 方法**（scheduler.rs:47）
   - Brief 描述为"8 个 async 方法"包含 `new`，但 brief verbatim 代码本身 `new` 也是 sync 的（`pub fn new() -> Self`）。
   - 实现与 brief verbatim 代码一致，不算偏差；只是 brief 文字描述与 verbatim 代码本身有出入。
   - 不需要在本 task 修复——`new` 不需要 await 任何东西，sync 构造是 idiomatic Rust。

2. **`seen_urls` 是 placeholder**（scheduler.rs:86-94）
   - 返回 `HashSet<String>`，内容是 `u64` 哈希值的 `to_string()`，不是真实 URL。
   - Brief 第 99 行注释明确说"For simplicity in stage 1, we store the full URL set here"——但代码实际只存了 hash。这是 brief 自身的设计缺陷，非 implementer 之责。
   - 已在"Known limitations"中列出，不作为问题。

3. **`pending_urls` 注释"Need Clone bound on PrioritizedRequest - add it"**（scheduler.rs:80）
   - 注释像是 TODO 留下的痕迹，实际已在文件末尾添加 `impl Clone for PrioritizedRequest`（scheduler.rs:128-132）。
   - Brief verbatim 代码就有这句注释，implementer 原样保留。
   - 微小可读性问题，非功能性，无需修复。

## Assessment

**Task quality:** Approved

**Reasoning:** 实现严格匹配 brief 的 verbatim 代码，仅在 `push`/`restore` 两处对 `g.seq` 做了最小且语义保持的 borrow 修复（brief 原代码无法通过 E0502）。`cargo check` 确认 scheduler.rs 零编译错误，mod.rs 的 1 个 `.await` 错误是 Task 8 的预期工作。`tokio::sync::Mutex` 选择正确，async 接口签名符合 `&self + Mutex` 内部可变性模式，去重 / 优先级 / restore 语义全部正确。
