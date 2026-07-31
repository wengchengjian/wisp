# Task 8 Review: Engine 重构为 buffer_unordered 并发

## 审查范围

- Diff: `602f76e..7b431d5`
- 文件: `src/crawl/mod.rs`（修改）、`tests/crawl_concurrency_test.rs`（新建）
- 审查依据: `task-8-brief.md`（修正版 brief）+ plan Architecture 段落全局约束
- 已交叉对照 plan 原始 Task 8 代码（`docs/superpowers/plans/2026-07-21-stage1-p0-hardfixes.md:1457-1776`），以判定问题是 brief 自身设计缺陷还是 implementer 引入

---

## Spec 合规

### Brief 步骤逐项

| Brief 要求 | 实现位置 | 状态 |
|---|---|---|
| Step 1: 创建 `tests/crawl_concurrency_test.rs` | 新建文件，36 行 | ✅ 与 brief verbatim 一致 |
| Step 2: imports 追加（atomic/Arc/futures/tokio::Mutex） | mod.rs:10-15 | ✅ 未重复 import HashMap/Duration |
| Step 2: 新增 `EngineConfig` + `Default` | mod.rs:105-115 | ✅ 字段/默认值与 brief 一致 |
| Step 2: `Engine` struct 改为 `spider + config` | mod.rs:117-121 | ✅ |
| Step 2: `new` 从 `concurrent_requests()` 初始化 | mod.rs:124-133 | ✅ |
| Step 2: `max_pages` / `max_concurrent` builder | mod.rs:135-136 | ✅ |
| Step 2: `run()` 提前提取所有 config + spider 信息 | mod.rs:141-146 | ✅ 避免 self 部分移动 |
| Step 2: `Arc::new(self.spider)` + sched/robots_cache Arc 化 | mod.rs:154-156 | ✅ |
| Step 2: `unbounded_channel` 回灌 follow | mod.rs:164 | ✅ |
| Step 2: `AtomicUsize` 统计 | mod.rs:165-167 | ✅ |
| Step 2: per-domain `HashMap<String, Arc<Semaphore>>` | mod.rs:170-171 | ✅ |
| Step 2: `stream::unfold` + `buffer_unordered` | mod.rs:190-297 | ✅（含 3 处偏离，见下） |
| Step 2: `fetch_page` 改为自由函数 | mod.rs:315-330 | ✅ 签名与 brief 一致 |
| Step 2: 保留 SpiderRequest/SpiderResponse/Spider trait/Method/CrawlStats | mod.rs:21-103 | ✅ 未触动 |
| Step 3: `cargo check` 通过 | report 附 exit 0 | ✅ |
| Step 4: `cargo test --lib` 通过 | report 附 34 passed | ✅ |
| Step 5: 提交 | 7b431d5 | ✅ |

### Plan 全局约束逐项

| Plan 约束 | 状态 | 说明 |
|---|---|---|
| `Engine::run` 重构为 `stream::unfold + buffer_unordered` | ✅ | mod.rs:190-297 |
| follow requests 通过 channel 回灌 scheduler | ⚠️ | channel 存在，但 unfold 终止条件导致 in-flight follow 可能丢失（见 Concern 1） |
| 保留现有 Spider trait 接口（不破坏 API） | ✅ | trait 方法全部保留；但 `download_delay` 不再被 Engine 调用（见 Concern 2） |
| per-domain 信号量做节流 | ✅ | mod.rs:170-171, 260-266（语义见 Concern 3） |

### 3 处 brief 代码偏离评估

| 偏离 | Brief 代码 | Implementer 修复 | 评估 |
|---|---|---|---|
| 1. 删除 `.map(\|(fut, _)\| fut)` | unfold closure 返回 `Some((fut, ()))`，Item 即 `fut`（async block），map 试图把 `fut` 当元组解构 | 直接删除 map 行 | ✅ 正确且最小。`stream::unfold` 的 closure 返回 `Option<(Item, State)>`，Item = `fut`，unfold 直接 yield `fut`。map 是 brief 笔误（plan 原文也有同一 bug） |
| 2. 加 `tokio::pin!(stream)` | `while stream.next().await.is_some() {}` | 前置 `tokio::pin!(stream);` 并去掉 `let mut` | ✅ 正确且最小。`BufferUnordered<Unfold<..., async block>>` 的 async block 是 `!Unpin`，导致 stream `!Unpin`，`StreamExt::next` 要求 `Self: Unpin` |
| 3. `let mut rc` | `let rc = robots_cache_c.lock().await;` | 改为 `let mut rc` | ✅ 正确且最小。`DerefMut::deref_mut(&mut self)` 需要 `&mut rc`，brief「常见编译问题 #1」已预见 |

**3 处偏离全部是正确的最小修复，针对 brief 代码的真实编译错误，无过度修改。**

---

## 代码质量 Findings

### Critical

#### C1: follow channel 丢消息（unfold 提前终止）

**位置**: `src/crawl/mod.rs:203-217`

```rust
async move {
    // 1. Drain follow channel into scheduler
    let mut rx_guard = follow_rx.lock().await;
    while let Ok(req) = rx_guard.try_recv() {
        sched.push(req).await;
    }
    drop(rx_guard);

    // 2. Check page budget
    if stats_pages.load(Ordering::SeqCst) >= max_pages {
        return None;
    }

    // 3. Pop next request
    let req = sched.pop().await?;  // ← None 时 unfold 返回 None，stream 终止
    ...
}
```

**问题**: 当 `buffer_unordered` buffer 未满时，它会持续 poll unfold。如果此时：
- scheduler 为空（所有种子/follow 已被 pop）
- channel 为空（in-flight future 尚未发出 follow）
- 但仍有 in-flight future 正在执行（将发出 follow）

则 `sched.pop().await?` 返回 `None`，unfold 返回 `None`，stream 被标记为 exhausted。此后 in-flight future 完成时发出的 follow 请求进入 channel，但 unfold 不再被 poll，channel 永远不会被 drain。`buffer_unordered` 会继续处理完剩余 in-flight future 后返回 `None`，主循环退出，follow 请求被丢弃。

**触发场景**（常见）:
- 单种子 URL + `max_concurrent > 1`：第一次 `stream.next()` 时 unfold 产出 1 个 future，buffer 还有空位，unfold 被再次 poll，sched 空 → 返回 None → stream 终止。future 完成后发出的所有 follow 丢失。
- 任何「种子数 < max_concurrent」或「follow 产出有延迟」的真实爬虫。

**为什么 brief 测试没触发**: 测试 spider 的 `parse` 返回 `(vec![], vec![])`（无 follow），且 10 个种子 ≥ max_concurrent=4，unfold 总能从 sched pop 到项目，不会提前返回 None。

**违反约束**: plan Architecture「follow requests 通过 channel 回灌 scheduler」——channel 回灌机制存在但无法可靠工作，in-flight follow 会被丢弃。

**修复方向**: 在 unfold 终止条件中加入 in-flight 计数：
```rust
// 用 Arc<AtomicUsize> 跟踪 in-flight future 数
// unfold 返回 None 当且仅当: sched.is_empty() && channel_empty && in_flight == 0
```
具体实现：
- 在 unfold 外维护 `in_flight: Arc<AtomicUsize>`
- 产出 future 时 `in_flight.fetch_add(1, SeqCst)`
- future 完成时（无论结果）`in_flight.fetch_sub(1, SeqCst)`
- unfold 的 `sched.pop().await?` 改为：若 `sched.is_empty()` 且 `in_flight == 0` 才返回 None，否则 `yield_now` + retry

### Important

#### I1: `download_delay` 功能丢失

**位置**: `src/crawl/mod.rs` 全文

**问题**: 原 Engine 在每次请求后 `tokio::time::sleep(self.spider.download_delay()).await`。新 Engine 完全不调用 `Spider::download_delay()`。trait 方法仍存在（mod.rs:85），但 Engine 不再使用。

**评估**:
- Plan 全局约束「保留现有 Spider trait 接口（不破坏 API）」——API 层面满足（方法仍在）。
- 但行为层面是回归：用户依赖 `download_delay` 做礼貌爬取会失效。
- Plan Task 8 代码（plan:1561-1730）本身就没有 `download_delay` 调用——这是 plan 级设计决策，非 implementer 引入。
- Plan Architecture「per-domain 信号量做节流」可解读为信号量替代 delay，但信号量限制的是并发数而非请求间隔，不是语义等价替代。

**建议**: 在 per-domain 信号量 acquire 后、fetch 前，加 `tokio::time::sleep(spider.download_delay()).await`，保留延迟语义；或在 trait 文档注明「并发模式下 `download_delay` 由 per-domain 信号量替代」。

#### I2: `robots_cache` Mutex 锁在 network I/O 期间持有

**位置**: `src/crawl/mod.rs:246-249`

```rust
let allowed = {
    let mut rc = robots_cache_c.lock().await;      // 全局锁
    rc.is_allowed(&client_r, &url_clone).await     // 含网络请求（fetch robots.txt）
};
```

**问题**: `RobotsCache::is_allowed` 是 `&mut self` + `async`，首次访问某域时会 `client.get(&robots_url).await` 拉取 robots.txt。期间 `robots_cache_c` 的 `Mutex` 锁被持有，所有其他请求的 robots 检查被阻塞。

**影响**: 10 个不同域的请求本可并发拉取 robots.txt，但因全局锁被序列化。每个 robots.txt 拉取若耗时 200ms，10 个域需 2s 串行等待，严重削弱并发收益。

**根因**: `RobotsCache::is_allowed` 设计上把「检查缓存」和「网络拉取」放在同一个 `&mut self` 调用中。Task 8 用 `Mutex` 包装后暴露此问题。

**注意**: 此模式来自 brief verbatim 代码（brief:218-221），非 implementer 引入。但 implementer 在并发场景下使用全局 `Mutex` 包装，放大了问题。

**建议**: 短期可在 `RobotsCache` 内部改用 `HashMap<String, Arc<Mutex<()>>>` per-domain 锁，或用 `tokio::sync::RwLock` + 双检（先读锁检查、miss 时升级写锁拉取）。长期应重构 `RobotsCache` 使网络拉取在锁外完成。

### Minor

#### M1: per-domain 信号量许可数 = `max_concurrent`，节流冗余

**位置**: `src/crawl/mod.rs:263`

每域 `Semaphore::new(max_concurrent)`，与 `buffer_unordered(max_concurrent)` 全局上限相同。单域可占满全局并发，多域时每域信号量永远不会先于全局 buffer 成为瓶颈。per-domain 节流实际未生效。

**评估**: per brief 设计（brief:235），非 implementer 偏差。若 plan 意图是「per-domain 并发独立于全局」，应增加 `per_domain_concurrent` 配置。当前实现可接受，但「per-domain 节流」名不副实。

#### M2: page budget 是软限制，实际可能超出 `max_pages`

**位置**: `src/crawl/mod.rs:212`（unfold 检查）vs `mod.rs:275`（future 递增）

unfold 检查 `stats_pages >= max_pages` 时，多个 future 可能已 in-flight 但尚未递增 `stats_pages`。最终 `pages_crawled` 可能超出 `max_pages` 最多 `max_concurrent - 1` 个。

**评估**: 并发处理中常见权衡，多数场景可接受。若需严格上限，应在 future 内 fetch 前 再次检查 `stats_pages` 的 CAS。

#### M3: `sem.acquire_owned().await.unwrap()` 不必要的 panic 风险

**位置**: `src/crawl/mod.rs:266`

信号量未被 close（`domain_sems` 与 `run()` 同生命周期），实际不会 panic。但 `unwrap()` 不符合生产代码风格。建议 `.expect("domain semaphore closed")` 或返回 `Result`。

#### M4: 测试标记 `#[ignore]`，并发行为未被 CI 验证

**位置**: `tests/crawl_concurrency_test.rs:26`

`#[ignore = "requires network access to httpbin.org"]` 意味着 `cargo test` 默认不运行。brief 如此要求，implementer 照做。但导致 C1（follow 丢失）和并发正确性完全无测试覆盖。建议后续 task 补充离线 mock 测试（用 `wiremock` 或本地 HTTP server）。

#### M5: 测试文件 `use std::sync::Arc;` 未使用

**位置**: `tests/crawl_concurrency_test.rs:3`

brief verbatim 代码自带的未使用 import。`cargo check --tests` 有 warning。建议删除该行（brief 原文瑕疵）。

#### M6: `domain.clone()` 可省略

**位置**: `src/crawl/mod.rs:262`

`sems.entry(domain.clone())` 中 `domain` 在后续未使用，可直接 `sems.entry(domain)` 移动所有权，省一次 `String` clone。微观优化，非功能性问题。

---

## Concerns 评估（独立判断）

### Concern 1: follow channel 丢消息

**Implementer 判断**: 架构性关切，建议后续 task 用 in-flight 计数修复。

**独立评估**: **确认是真实 Critical 问题。**

- 触发条件明确：sched 空 + channel 空 + in-flight future 存在时 unfold 返回 None。
- 影响面：任何有多层 follow 的真实爬虫（非测试 spider）都会丢消息。单种子 + max_concurrent>1 的最常见场景即触发。
- 违反 plan「follow requests 通过 channel 回灌 scheduler」约束——机制存在但不可靠。
- 不属于「合理简化」或「阶段 1 占位」，因为 plan 明确要求 channel 回灌，且回灌不可靠等于功能失效。
- 修复成本可控：`Arc<AtomicUsize>` in-flight 计数 + unfold 终止条件改为三条件 AND。

**Severity: Critical**（必须在本 task 或紧邻 fix task 修复，不能拖到阶段 2/3）

### Concern 2: `download_delay` 功能丢失

**Implementer 判断**: 功能丢失，建议加回 sleep 或文档说明。

**独立评估**: **确认是 Important 问题，但属 plan 级设计决策。**

- Plan Task 8 代码（plan:1561-1730）本身就没有 `download_delay` 调用——implementer 是忠实执行 plan/brief，未擅自删除。
- Trait API 保留（`download_delay` 方法仍在），符合「不破坏 API」字面要求。
- 但行为回归真实存在：用户若覆盖 `download_delay` 期望生效，会静默失效。
- Plan「per-domain 信号量做节流」可解读为替代方案，但信号量（并发上限）与 delay（请求间隔）语义不等价。

**Severity: Important**（建议在本 task 或 fix task 补回 `sleep` 调用，或在 trait 文档注明语义变化）

### Concern 3: per-domain 信号量许可数 = `max_concurrent`

**Implementer 判断**: 按 brief 实现，语义待 reviewer 确认。

**独立评估**: **Minor 设计问题，per brief，可接受。**

- 每域信号量许可 = 全局 max_concurrent，意味着 per-domain 节流实际未生效（全局 buffer 先成为瓶颈）。
- Plan Architecture「per-domain 信号量做节流」的字面意图是 per-domain 独立于全局的节流，当前实现不满足此意图。
- 但此为 plan/brief 设计层面问题，非 implementer 引入。阶段 1 可接受，后续可增加 `per_domain_concurrent` 配置项。

**Severity: Minor**（不阻塞，记录为已知简化）

---

## 总评

**APPROVED with Critical fix required** → **NEEDS_FIX**

实现严格遵循修正版 brief，3 处偏离全部是正确的最小编译修复，cargo check/test 均通过。但 brief 自身存在一个 Critical 设计缺陷（follow channel 丢消息），导致 plan「follow requests 通过 channel 回灌 scheduler」约束未被有效满足。该缺陷在 brief 测试（无 follow spider）下不可见，但对真实爬虫是功能失效。

### Fix subagent 需处理的 findings

| Finding | Severity | 修复要求 |
|---|---|---|
| C1: follow channel 丢消息 | Critical | 加入 `Arc<AtomicUsize>` in-flight 计数，unfold 终止条件改为 `sched.is_empty() && channel_empty && in_flight == 0`。需偏离 brief 代码（brief 未设计 in-flight 跟踪） |
| I1: download_delay 丢失 | Important | 在 per-domain 信号量 acquire 后、fetch 前加 `tokio::time::sleep(spider.download_delay()).await`。需从 `Arc<S>` 上调用 `spider.download_delay()` |
| I2: robots_cache 锁跨 await | Important | 可选修复：改用 per-domain 锁或双检模式。若不修，需在代码注释标注已知性能问题 |
| M5: 测试未使用 import | Minor | 删除 `tests/crawl_concurrency_test.rs:3` 的 `use std::sync::Arc;` |

### 可不处理（记录为已知简化）

- M1: per-domain 信号量冗余（per brief 设计）
- M2: page budget 软限制（并发常见权衡）
- M3: `unwrap()` 风格（实际不会 panic）
- M4: 测试 `#[ignore]`（per brief，后续 task 补离线测试）
- M6: `domain.clone()` 微优化
- Concern 3: per-domain 信号量许可数（per brief 设计）

---

## 补充说明

- C1 的修复需要偏离 brief 代码（brief 未设计 in-flight 跟踪）。建议 fix subagent 在 `task-8-fix-brief.md` 或等价文档中记录此偏离的理由和实现方案。
- 交叉对照 plan 原始 Task 8 代码（plan:1457-1776）发现，plan 原版比修正版 brief 更破碎（skip 路径返回 `()` 与 fetch 路径返回 `impl Future` 类型不匹配，`.map(\|(fut, _)\| fut)` 同样存在）。修正版 brief 已修复类型不匹配问题（统一为 `async move` block），implementer 进一步修复了剩余 3 处编译错误。修复链条合理。

---

## Re-Review (Fix Round 1)

**审查范围**: `7b431d5..0b4e731`（Fix Round 1 diff）+ `602f76e..0b4e731`（整体回归）
**HEAD**: `0b4e731`
**审查依据**: `task-8-review.md` 原 findings + `task-8-brief.md`

### Fix 验证

#### C1 (Critical): follow channel 丢消息 — ✅ 通过

**实现验证**（`src/crawl/mod.rs`）:

1. **in-flight 计数器** (line 178): `let in_flight = Arc::new(AtomicUsize::new(0));` ✓
2. **递增点** (line 244): `in_flight.fetch_add(1, Ordering::SeqCst);` — 在构造 future 前、pop 成功后 ✓
3. **RAII guard** (line 356-370): `InFlightGuard` struct + `Drop` impl，持有 `Arc<AtomicUsize>` clone（避免自引用），`Drop::drop` 执行 `fetch_sub(1, SeqCst)` ✓
4. **guard 创建位置** (line 260): `let _guard = InFlightGuard { counter: in_flight_c };` — future 内首行 ✓

**退出路径覆盖验证**（所有路径都触发 guard 的 Drop）:
- Domain filter skip (line 267 `return`) ✓
- Robots disallow (line 285 `return`) ✓
- is_blocked (line 313 `return`) ✓
- Fetch error (line 326-329，match arm 末尾) ✓
- Normal completion (line 331，future 末尾) ✓
- Panic 路径: Rust unwind 语义保证 Drop 执行 ✓

**unfold 终止条件验证** (line 209-335，`loop` 结构):
- Budget reached + `in_flight == 0` → `return None` (line 222-224) ✓
- Budget reached + `in_flight > 0` → `yield_now + continue` (line 225-226) ✓
- `sched.pop() → None` + `in_flight == 0` → `return None` (line 235-237) ✓
- `sched.pop() → None` + `in_flight > 0` → `yield_now + continue` (line 238-239) ✓
- `sched.pop() → Some(req)` → `fetch_add(1)` + 构造 future + `return Some((fut, ()))` ✓

**yield_now 是否 busy loop**: 不是。`yield_now` 将当前任务放回队列尾部，让 executor 处理其他就绪任务。in-flight future 内含 `await` 点（网络 I/O / sleep），其 waker 注册到 `buffer_unordered` 的 waker。当 I/O 就绪时唤醒主任务，`buffer_unordered.poll_next` 会先 poll in-flight future（完成则 drop guard），再 poll unfold（此时 `in_flight` 已递减）。循环由 in-flight future 完成驱动，非 CPU 空转。可接受。

**budget 达上限行为**: budget reached 时不再 pop 新请求，in-flight future 完成后其 follow 请求进入 channel → 被 drain 进 sched → 但 sched 不会被 pop（budget reached）→ `in_flight == 0` 时 unfold 返回 None，sched 中剩余 follow 被丢弃。这与原串行 Engine `break` 后丢弃 sched 中 follow 的行为一致 ✓

#### I1 (Important): download_delay 丢失 — ✅ 通过

**位置验证** (line 300-306):
```rust
let _permit = sem.acquire_owned().await.unwrap();  // 信号量 acquire 后
// I1 fix: download_delay - per-domain 信号量 acquire 后、fetch 前
let delay = spider_clone.download_delay();
if delay > Duration::ZERO {
    tokio::time::sleep(delay).await;
}
// 7. Fetch
match fetch_page(&client_c, &req).await {           // fetch 前
```
位置正确：acquire 后、fetch 前 ✓

**Arc<S> 调用验证**: `spider_clone` 是 `Arc<S>`（line 245），`Spider::download_delay(&self)` 是 trait 方法，`Arc<S>` 通过 `Deref<Target = S>` 自动解引用调用 ✓

**`Duration::ZERO` 守卫**: 默认 `download_delay` 返回 `Duration::from_millis(0)`，`delay > Duration::ZERO` 避免无意义 sleep ✓

#### I2 (Important): robots_cache 锁跨 await — ✅ 通过

**注释验证** (line 274-276):
```rust
// NOTE (I2): robots_cache 的全局 Mutex 在 is_allowed 的网络拉取
// 期间被持有，序列化所有域的 robots 检查。阶段 1 接受此性能限制，
// 后续可改为 per-domain 锁或在 RobotsCache 内部双检。
```
注释清晰说明: 问题（全局 Mutex 持有期间含网络拉取）+ 影响（序列化所有域）+ 接受决策（阶段 1）+ 改进方向（per-domain 锁 / 双检）✓

符合原 review「可选修复：若不修，需在代码注释标注已知性能问题」要求 ✓

#### M5 (Minor): 测试未使用 import — ✅ 通过

**删除验证**: `tests/crawl_concurrency_test.rs:3` 现为 `use async_trait::async_trait;`，`use std::sync::Arc;` 已删除 ✓

**编译验证**: `cargo check --tests` exit 0，`crawl_concurrency_test.rs` 的 unused import warning 已消失，仅剩 `adaptive_test.rs:58` 预先存在的 `unused variable: store` warning ✓

### 构建与测试验证

- `cargo check --tests`: exit 0，无新增 warning（5 个 lib warning + 1 个 adaptive_test warning 均为预先存在）
- `cargo test --lib`: exit 0，`test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`，未破坏现有功能

### 整体回归审查（602f76e..0b4e731）

对比完整 diff，Fix Round 1 改动仅涉及:
- `src/crawl/mod.rs`: unfold 内部重写为 `loop` + in_flight 跟踪 + I1 sleep + I2 注释 + InFlightGuard struct
- `tests/crawl_concurrency_test.rs`: 删除 1 行 import

未触及 `Spider` trait / `SpiderRequest` / `SpiderResponse` / `CrawlStats` / `fetch_page` 签名 / `Engine` public API（`new` / `max_pages` / `max_concurrent` / `run`）。无 API 回归。未引入新依赖。未修改 brief 范围外文件。

### 新 Concerns 评估

1. **C1 yield_now 而非 sleep** — 可接受。`yield_now` 是 cooperative 调度，依赖 in-flight future 的 `await` 点驱动唤醒，非 busy spin。生产场景下若 in-flight future 长时间无 I/O 就绪（极端超时场景），会有少量额外 polling，但 tokio 默认超时会最终触发唤醒。阶段 1 可接受，无需改为 `sleep(1ms)`。

2. **Budget 达上限后 in-flight future 仍递增 stats_pages（M2）** — 可接受。原 review 已标记 M2 为「可不处理」（并发常见权衡，最多超出 `max_concurrent - 1` 个）。Fix 未恶化此行为。

3. **I2 仅注释未重构 RobotsCache** — 符合 fix 范围。原 review 明确「可选修复：若不修，需在代码注释标注」。注释内容准确，阶段 1 接受。

4. **未跑 crawl_concurrency_test** — 可接受。该测试 `#[ignore]` 且需 httpbin.org 网络访问，按 brief 约束不执行。C1 正确性通过代码审查验证（RAII guard 覆盖所有退出路径 + 终止条件三要素 AND）。建议后续 task 用 wiremock 补离线并发测试（对应原 M4）。

### 总评

**APPROVED**

Fix Round 1 正确修复了全部 4 个 findings（C1/I1/I2/M5），实现质量高:
- C1 的 RAII guard 设计稳健，覆盖所有退出路径（含 panic unwind），`yield_now` 非 busy loop
- I1 位置精确（acquire 后 fetch 前），正确通过 `Arc<S>` deref 调用
- I2 注释完整标注问题/影响/接受/改进方向
- M5 已删除，warning 消失

构建与测试全绿（34 lib tests passed），无新增 warning，无 API 回归，未引入新依赖。4 个新 concerns 全部可接受（均符合原 review 的「可不处理」或 fix 范围约定）。

**Fix Round 1 next BASE**: `0b4e731202082d7ff0255b1d52bb5009ed6569f0`
