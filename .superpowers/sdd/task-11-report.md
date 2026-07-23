# Task 11 Report: 修复 tracker std::sync::Mutex 中毒 panic

## 状态
DONE

## Commit
- Hash: `801fa658494891ab94e0e57a78c1a2f3db963785`
- Branch: `fix/code-review-2026-07-23`
- Message:
  ```
  fix(crawl): tracker Mutex 中毒时不再二次 panic

  - css/xpath_auto/auto_upgrade_check 用 unwrap_or_else(into_inner) 处理中毒锁
  - 另一 task panic 持锁时，当前 task 取数据而非 panic 传播
  ```

## 修改文件
- `src/crawl/mod.rs`
  - L158（`SpiderResponse::css`）：`t.lock().unwrap()` → `t.lock().unwrap_or_else(|e| e.into_inner())`
  - L167（`SpiderResponse::xpath_auto`）：同上
  - L484-506：新增测试 `spider_response_css_with_tracker_does_not_panic`（追加在 `#[cfg(test)] mod tests` 末尾）
- `src/crawl/engine.rs`
  - L440（`auto_upgrade_check`）：`tracker.lock().unwrap()` → `tracker.lock().unwrap_or_else(|e| e.into_inner())`

## 实际行号 vs brief
brief 标注 `mod.rs:150-152, 159-161` 与 `engine.rs:438`。实际源码因前序 task 改动略有偏移：
- `mod.rs` css 方法 tracker 调用在 L158（非 L151）
- `mod.rs` xpath_auto 方法 tracker 调用在 L167（非 L160）
- `engine.rs` auto_upgrade_check tracker 调用在 L440（非 L438）

修改按实际行号定位，未受 brief 偏移影响。

## TDD 流程
1. **RED/GREEN 基线**：先在 `mod.rs` 测试模块末尾追加 `spider_response_css_with_tracker_does_not_panic` 测试（构造带 tracker 的 `SpiderResponse`，调 `css("p")`，断言节点数 1、`t.len()==1`、`t.records().len()==1`）。运行 `cargo test --lib crawl::tests::spider_response_css_with_tracker_does_not_panic` → **PASS**（符合 brief 预期：这是不回归测试，非 RED）。
2. **修改三处 `lock().unwrap()` → `lock().unwrap_or_else(|e| e.into_inner())`**（mod.rs css / mod.rs xpath_auto / engine.rs auto_upgrade_check），单行保持。
3. **验证**：
   - `cargo test --lib crawl::tests::spider_response_css_with_tracker_does_not_panic` → 1 passed
   - `cargo test --lib` → **205 passed; 0 failed; 0 ignored**
   - `cargo build --lib` → Finished（仅预存在 warning，无 error）

## 测试命令与输出摘要
- `cargo test --lib crawl::tests::spider_response_css_with_tracker_does_not_panic 2>&1 | tail -10`
  → `test crawl::tests::spider_response_css_with_tracker_does_not_panic ... ok` / `test result: ok. 1 passed; 0 failed`
- `cargo test --lib 2>&1 | tail -20`
  → `test result: ok. 205 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`
- `cargo build --lib 2>&1 | tail -10`
  → `Finished dev profile [unoptimized + debuginfo] target(s)`（7 个预存在 warning，无 error）
- `git log --oneline -2`
  → `801fa65 fix(crawl): tracker Mutex 中毒时不再二次 panic` / `17d0716 fix(browser): 代理认证丢失改为显式告警`

## 担忧与观察
- 无功能担忧。`unwrap_or_else(|e| e.into_inner())` 是 `std::sync::Mutex` 处理中毒锁的官方惯用法（`PoisonError::into_inner` 返回被守护的 `MutexGuard`），语义为「另一 task panic 持锁时，当前 task 仍取数据继续」，与 brief 期望一致。
- 测试为不回归测试（brief 明示难以注入中毒锁），未直接验证「中毒时不 panic」路径。这是 brief 既定设计，非缺陷。
- 改动行内未引入新 warning；`cargo build --lib` 的 7 个 warning 均为预存在（`unused import` 等），与本 task 无关。
- 仅修改 brief 列出的两个文件，未触碰其他代码；未新增字段/方法/功能。
