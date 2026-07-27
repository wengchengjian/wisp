# Final Review Fix Report

**Branch:** `feat/engine-ergonomic-helpers`
**Base HEAD:** `575473d`
**Fix commit:** `eaa95a2` — `fix: 应用 final review 反馈（移除 idx 参数、补 trace 日志、文档示例与注释自包含）`

## 改动清单

### Fix I-1: 移除 `enqueue_links_with` 的 `idx` 参数（YAGNI）

**文件:**
- `src/fetcher/response.rs:525-567`（签名 + 实现）
- `src/fetcher/response.rs:512`（`enqueue_links` 内部闭包 `|_| Some(Value::Null)`）
- `src/fetcher/response.rs:903`（test `test_enqueue_links_with_injects_meta_from_closure`）
- `src/fetcher/response.rs:934`（test `test_enqueue_links_with_skips_when_closure_returns_none`）
- `examples/novel_crawler.rs:68`（default handler 闭包 `|a|`）

**改动:** 闭包签名 `Fn(&crate::parser::Node, usize) -> Option<Value>` → `Fn(&crate::parser::Node) -> Option<Value>`；实现 `for (idx, a) in links.iter().enumerate()` → `for a in links.iter()`；`meta_fn(a, idx)` → `meta_fn(a)`；所有调用点闭包从 `|a, _idx|` 改为 `|a|`。doc comment 中移除「及其在当前 selector 结果中的索引（`usize`）」一句。

### Fix I-3: `follow_with` URL 解析失败时补 trace 日志

**文件:** `src/fetcher/response.rs:552-555`

**改动:** 在 `let Some(req) = self.follow_with(&href, callback) else { ... }` 的 else 分支中新增 `tracing::trace!("enqueue_links: 跳过无法解析的 href: {:?}", href);`。`href` 类型为 `String`（来自 `Node::attr` 返回 `Option<String>`），`{:?}` Debug 格式合法。

### Fix M-1: `title_for_log` 避免不必要的 String 分配

**文件:** `src/crawl/engine.rs:252-256`

**改动:** 原代码 `resp.title().unwrap_or_default().to_string()` 每次都分配一个 `String`。

**实现偏差（重要）:** 指令建议改为 `let title_for_log: &str = resp.title().unwrap_or("");`，但这会触发 borrow checker 错误 E0505 —— `title_for_log` 借用 `resp`，而 `resp` 紧接着在 `spider.handle(resp).await` 被移动，借用延伸到移动之后（log 调用点）。

为实现零分配目标，采用 `mem::take` 模式：
```rust
let title_owned = resp.title.take();
let title_for_log: &str = title_owned.as_deref().unwrap_or("");
```
`title_owned` 是局部 `Option<String>`，`title_for_log` 借用局部变量而非 `resp`，借用不延伸过 move。

**语义代价:** `resp.title` 字段在传入 `spider.handle(resp)` 前被清空为 `None`。审查结论：
- `Response::title` 字段文档标注「浏览器模式下的页面标题」——诊断字段，非业务数据
- 全代码库无 spider handler 读取 `resp.title()`（`novel_crawler.rs` 三个 handler 均使用 `meta_str`/`parse()`）
- `process_response` 是 title 字段的唯一消费者
- 若未来有 spider handler 需读取浏览器标题，需重新评估此优化

`tracing::warn!`/`info!` 调用点保持 `title={:?}` 不变（`&str` 实现 Debug，编译通过）。

### Fix M-2: detail handler 注释自包含

**文件:** `examples/novel_crawler.rs:96-98`

**改动:** 移除「（详见 plan Step 8 结论）」引用，替换为自包含解释：
```rust
// 章节列表（常见选择器）—— 此处需同时提取作者 + 章节列表，
// 调用 enqueue_links_with 会触发第二次 parse 而 panic
// （Response 单次解析约束），因此保留显式手写循环。
```

### Fix M-3: `enqueue_links` 跳过 `with_meta` 断言

**文件:** `src/fetcher/response.rs:825-826`

**改动:** 在 `test_enqueue_links_returns_follows_for_first_matching_selector` 末尾新增：
```rust
// 验证 meta == Null 时跳过 with_meta，使用 Request 默认 meta
assert_eq!(follows[0].meta, serde_json::Value::Null);
```

### Fix M-5: `meta_str` / `meta_u64` 补 `# Examples` doc-test

**文件:** `src/fetcher/response.rs:458-494`

**改动:** 为两个方法各新增 `# Examples` 段，使用 `no_run` + 隐藏 `# use wisp::Response;` + `# fn example(resp: &Response) { ... }` 包裹，确保 doc-test 编译通过。

`meta_str` 之前的 typo（".meta 非 Object"）已在 Task 3 修复为「meta 非 Object」，本次未重复修复。

### Fix M-6: 文档化「全部过滤掉也停止回退」语义

**文件:** `src/fetcher/response.rs:483-484`（`enqueue_links`）、`src/fetcher/response.rs:519-520`（`enqueue_links_with`）

**改动:** 两个方法的 doc comment 中均新增：
```
/// 注意：即使该 selector 的所有匹配都被空值过滤掉，回退也会停止——
/// 不会自动尝试下一个 selector。
```

### Fix I-2 / I-4: plan Self-Review 补「实际成果」与 Deferred 项

**文件:** `docs/superpowers/plans/2026-07-27-engine-ergonomic-helpers.md:840-855`

**改动:** 在 `## Self-Review` 段顶部（`### Spec coverage` 之前）插入两个新子节：
- `### 实际成果（post-implementation）`：含 6 行指标对比表 + 未达成「~40 行」目标的原因说明
- `### Deferred 项`：标注 I-4 集成测试缺失，建议作为独立 PR

**重要说明:** `docs/` 在 `.gitignore` 第 8 行被忽略，plan 文件从未被 git 跟踪（`git log -- docs/superpowers/plans/...` 返回空），本分支前 3 个 commit 也未跟踪该文件。本次改动已写入磁盘（`grep 实际成果` 确认 1 处命中），但未 `git add -f` 进入 commit，以尊重分支既定约定。如需纳入版本控制，可后续 `git add -f docs/superpowers/plans/2026-07-27-engine-ergonomic-helpers.md`。

## 测试验证

| 命令 | 结果 |
|---|---|
| `cargo test --workspace --lib --bins` | ✅ 291 passed; 0 failed; 0 ignored |
| `cargo build --example novel_crawler` | ✅ Finished `dev` profile in 3.73s |
| `cargo test --doc --package wisp` | ✅ 13 passed; 0 failed; 4 ignored（含新增 `meta_str`/`meta_u64` 两个 compile-only doc-test） |

## 关注点

1. **M-1 实现偏差:** 指令建议的 `let title_for_log: &str = resp.title().unwrap_or("");` 因 borrow checker（E0505：`resp` 在下一行被 move）无法直接编译。改用 `mem::take` 模式实现零分配，但代价是 `resp.title` 字段在传入 spider handler 前被清空。已在代码注释和本报告说明。若控制器不接受此语义代价，可回退为原始 `.to_string()` 分配（每次请求 1 次 String 分配，仅在诊断日志路径）。

2. **plan 文件未入 commit:** `docs/` 在 `.gitignore` 中，plan 文件改动仅在磁盘上生效，未进入 git 历史。与分支前 3 个 commit 的处理方式一致。

3. **Type consistency 段未同步更新:** plan Self-Review 的 `### Type consistency` 段仍提及旧的 `Fn(&crate::parser::Node, usize) -> Option<Value>` 签名（Step 7 扩展 idx 参数）。指令仅要求插入新子节，未要求修改既有段落，故未触碰。如需保持一致可后续修订。
