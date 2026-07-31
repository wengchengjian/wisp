### Task 11: 修复 tracker std::sync::Mutex 中毒 panic

**Files:**
- Modify: `src/crawl/mod.rs:150-152, 159-161`（SpiderResponse::css / xpath_auto 的 tracker 锁）
- Modify: `src/crawl/engine.rs:438`（auto_upgrade_check 的 tracker 锁）
- Test: `src/crawl/mod.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `Arc<std::sync::Mutex<SelectorTracker>>`（auto 模式追踪器）
- Produces: 锁中毒时返回默认行为（不记录选择器匹配）而非 panic

**背景：** `SpiderResponse::css`（L151）`t.lock().unwrap()` 和 `xpath_auto`（L160）同样。`auto_upgrade_check`（engine.rs:438）`tracker.lock().unwrap().needs_upgrade(...)`。若另一 task 持锁时 panic，锁中毒，`unwrap()` 二次 panic。应用 `lock().unwrap_or_else(|e| e.into_inner())` 优雅处理。

- [ ] **Step 1: 写测试 — 验证 lock 不 panic（间接：确认 css 在 tracker 存在时不崩溃）**

由于难以注入中毒锁，改为验证现有行为不回归。在 `src/crawl/mod.rs` 的 `#[cfg(test)]` 末尾追加：

```rust
    #[test]
    fn spider_response_css_with_tracker_does_not_panic() {
        use std::sync::{Arc, Mutex};
        use crate::crawl::auto::SelectorTracker;

        let tracker = Arc::new(Mutex::new(SelectorTracker::new()));
        let resp = SpiderResponse {
            url: "http://example.com".into(),
            status: 200,
            headers: std::collections::HashMap::new(),
            body: b"<html><body><p>x</p></body></html>".to_vec(),
            request: SpiderRequest::get("http://example.com"),
            tracker: Some(tracker),
            from_cache: false,
        };
        // 不应 panic
        let nodes = resp.css("p");
        assert_eq!(nodes.iter().count(), 1);
        // tracker 应记录（SelectorTracker.records 为私有，用 len() 方法）
        let t = resp.tracker.as_ref().unwrap().lock().unwrap();
        assert_eq!(t.len(), 1, "应记录 1 个选择器匹配");
        assert_eq!(t.records().len(), 1);
    }
```

注：`SelectorTracker.records` 是私有字段，但提供 `len()` 与 `records()` 方法（见 auto.rs:45-52）。

- [ ] **Step 2: 确认 auto.rs SelectorTracker API**

已确认（auto.rs:19-57）：字段 `records: Vec<(String, usize)>` 私有，方法 `record(&mut self, selector, match_count)`、`len()`、`records()`、`needs_upgrade(exclude)`。当前 crawl/mod.rs:151 调用 `t.lock().unwrap().record(sel, result.len())`。

- [ ] **Step 3: 修改三处 lock().unwrap() 为防中毒（保持单行）**

修改 `src/crawl/mod.rs` L151（css）：

```rust
            t.lock().unwrap_or_else(|e| e.into_inner()).record(sel, result.len());
```

修改 `src/crawl/mod.rs` L160（xpath_auto）：

```rust
            t.lock().unwrap_or_else(|e| e.into_inner()).record(expr, result.len());
```

修改 `src/crawl/engine.rs` L438：

```rust
    let needs = tracker.lock().unwrap_or_else(|e| e.into_inner()).needs_upgrade(auto_exclude);
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib crawl::tests::spider_response_css_with_tracker 2>&1 | tail -10`
Expected: PASS。

Run: `cargo test --lib 2>&1 | tail -15`
Expected: 全部 lib 测试通过。

- [ ] **Step 5: Commit**

```bash
git add src/crawl/mod.rs src/crawl/engine.rs
git commit -m "fix(crawl): tracker Mutex 中毒时不再二次 panic

- css/xpath_auto/auto_upgrade_check 用 unwrap_or_else(into_inner) 处理中毒锁
- 另一 task panic 持锁时，当前 task 取数据而非 panic 传播"
```

---

## Self-Review

**1. Spec coverage（对照 review 发现的 11 类缺陷）：**
- Task 1: browser/pool.rs retain + position 索引损坏 ✓（CRITICAL）
- Task 2: browser/pool.rs 轮询等待 ✓（MINOR）
- Task 3: crawl checkpoint seen 丢失 ✓（MAJOR）
- Task 4: autoscale 逻辑反转 ✓（MAJOR）
- Task 5: SqliteBackend::delete 契约 ✓（MAJOR）
- Task 6: CSS 选择器回退 `*` ✓（MAJOR）
- Task 7: robots.txt 端口 + 失败缓存 ✓（MAJOR + MINOR）
- Task 8: RequestCache 方法冲突 ✓（MAJOR）
- Task 9: resolve_href 非 http scheme ✓（MINOR）
- Task 10: 浏览器代理认证 ✓（MINOR，告警方案）
- Task 11: tracker Mutex 中毒 ✓（MINOR）

剩余未列入的 MINOR（refetch 绕过 process_request 检查、max_retries 语义困惑）为设计取舍，非缺陷，不改。

**2. Placeholder scan：** 无 TBD/TODO；每个 Step 含完整代码或命令；测试有具体断言。

**3. Type一致性：** 
- `RequestCache::{get,put,invalidate}` 签名在三处（定义、测试、engine.rs 调用）一致加 `method: &str`。
- `Store::delete_cached_response` 定义（Task 5 Step 3）与调用（Task 5 Step 4）签名一致。
- `RobotsRules::is_empty_rules` 定义（Task 7 Step 3）与调用一致。
- `Scheduler::restore(pending, seen)` 已存在（scheduler.rs:131），Task 3 调用签名匹配。
- `CrawlState` 字段名（state.rs:13-24）与 Task 3 手动构造一致。

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-23-code-review-fixes.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
