# Task 3 Report: Response 添加 enqueue_links helper

## 实现内容

### 1. `Response::enqueue_links` 与 `Response::enqueue_links_with`

在 `src/fetcher/response.rs` 的 `meta_u64` 之后追加两个方法（直接采用 plan Step 7 最终签名，跳过 Step 6/7 中间形态）：

- `enqueue_links(&self, selectors: &[&str], callback: &str) -> Vec<Request>`：批量提取链接的便捷方法，内部委托给 `enqueue_links_with`，传入常量闭包 `|_, _| Some(Value::Null)`。
- `enqueue_links_with<F>(&self, selectors: &[&str], callback: &str, meta_fn: F) -> Vec<Request> where F: Fn(&crate::parser::Node, usize) -> Option<Value>`：
  - 单次 `self.parse()`（避免触发单次解析断言）
  - 多选择器回退：`for sel in selectors` + `break`，匹配到第一个非空 selector 即停止
  - 空值过滤：跳过 `href` 为空或 `text().trim().is_empty()` 的 `<a>`
  - 闭包注入：`meta_fn(a, idx)` 返回 `None` 跳过；返回 `Some(Value::Null)` 走 `req`（不覆盖默认 meta）；返回 `Some(非 Null)` 走 `req.with_meta(meta)`
  - callback 自动设置：内部调用 `follow_with(&href, callback)`
  - `usize` 索引参数支持 `chapter_index` 场景

### 2. 默认 handler 重构（`examples/novel_crawler.rs`）

- **default handler**：从 52 行手写循环压缩到 20 行，调用 `resp.enqueue_links_with(..., |a, _idx| { ... })`
- **detail handler**：保留显式 `resp.parse()` + 手写循环模式（按 brief Step 8 结论）。原因：detail handler 需同时从 `.txt ul:nth-child(1)` 提作者 + 从 `.list ul li .name a` 提章节列表，调用 `enqueue_links_with` 会触发第二次 `parse()` 而 panic。这是 brief 文档化的诚实权衡。
- 同时把原 `resp.follow_meta(&href, json!({...})).map(|r| r.with_callback("detail"))` 模式替换为 `enqueue_links_with` 内部的 `follow_with + with_meta` 等价路径。

### 3. Task 2 的 2 个 Minor 文档错字修复

在 `src/fetcher/response.rs` 同一文件中：
- L458 `meta_str` 文档：`.meta 非 Object` → `meta 非 Object`（去掉多余的 `.`）
- L469 `meta_u64` 文档：`缺失/类型不符时返回 0` → `缺失/类型不符/meta 非 Object 时返回 0`（与 `meta_str` 对称）

## TDD 证据

### RED（实现前）

命令：`cargo test --package wisp --lib fetcher::response::tests::test_enqueue_links`

输出片段（编译错误）：
```
error[E0599]: no method named `enqueue_links` found for struct `response::Response` in the current scope
   --> src/fetcher/response.rs:723:28
    |
211 | pub struct Response {
    | ------------------- method `enqueue_links` not found for this struct
...
723 |         let follows = resp.enqueue_links(
    |                       -----^^^^^^^^^^^^^ method not found in `response::Response`

error[E0599]: no method named `enqueue_links_with` found for struct `response::Response` in the current scope
   --> src/fetcher/response.rs:809:28
```

### GREEN（实现后）

命令：`cargo test --package wisp --lib fetcher::response::tests::test_enqueue_links`

输出：
```
running 7 tests
test fetcher::response::tests::test_enqueue_links_returns_empty_when_no_selector_matches ... ok
test fetcher::response::tests::test_enqueue_links_falls_back_to_next_selector_when_first_empty ... ok
test fetcher::response::tests::test_enqueue_links_with_skips_when_closure_returns_none ... ok
test fetcher::response::tests::test_enqueue_links_returns_follows_for_first_matching_selector ... ok
test fetcher::response::tests::test_enqueue_links_with_injects_meta_from_closure ... ok
test fetcher::response::tests::test_enqueue_links_skips_empty_href_and_empty_text ... ok
test fetcher::response::tests::test_enqueue_links_panics_if_parse_already_called - should panic ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 284 filtered out; finished in 0.03s
```

### 修复 brief 测试用例 HTML bug

实现后第一次跑 GREEN 时，`test_enqueue_links_falls_back_to_next_selector_when_first_empty` 失败（left=0, right=1）。原因：brief 测试用例 HTML 写的是 `<ul class="list2">`，但 selector 是 `.list2 ul li .name a`（要求 `.list2` 是 div 容器内嵌 `<ul>`）。selector 来自 `examples/novel_crawler.rs` 的真实选择器列表（保留不变更合理），故修正 HTML 为 `<div class="list2"><ul>...</ul></div>` 以匹配 selector 语义。修正后该测试通过。

## 文件变更

### `src/fetcher/response.rs`
- L458: `meta_str` 文档错字修复
- L469: `meta_u64` 文档错字修复
- L478-564: 新增 `enqueue_links` 和 `enqueue_links_with` 实现
- L708-865: 新增 7 个测试用例

### `examples/novel_crawler.rs`
- L55-78: default handler 重构为 `enqueue_links_with`（52 行 → 20 行）
- L79-120: detail handler 重构，使用显式 `resp.parse()` + 手写循环 + `follow_with` + `with_meta`，并补注释说明为何不使用 `enqueue_links_with`

## Self-Review

### 完整性
- ✅ 7 个测试全部通过
- ✅ `enqueue_links` 和 `enqueue_links_with` 都用最终 Step 7 签名 `Fn(&Node, usize) -> Option<Value>`
- ✅ default handler 重构使用 `enqueue_links_with`（闭包 `|a, _idx|`）
- ✅ detail handler 使用显式 `parse()` + 手写循环（不使用 `enqueue_links_with`）
- ✅ Task 2 的 2 个文档错字已修复

### 质量
- ✅ `enqueue_links_with` 内单次 `self.parse()` 调用
- ✅ 多选择器回退：`for sel in selectors { if !links.is_empty() { ...; break; } }`
- ✅ 空 href 和空/空白 text 均被过滤
- ✅ `Value::Null` meta 走 `req`（不调用 `with_meta`）
- ✅ 文档注释全部中文

### 纪律
- ✅ 未添加 `enqueue_links_filtered`/`enqueue_links_strict` 等未要求的方法
- ✅ 未修改 `Node::select`/`Node::attr` 等现有方法
- ✅ 未碰 `tests/*.rs` 集成测试

### 测试
- ✅ RED：实现前编译错误
- ✅ GREEN：实现后 7 测试通过
- ✅ `cargo build --example novel_crawler` 通过
- ✅ `cargo test --workspace --lib --bins` 通过（291 passed; 0 failed）

## 最终测试命令与结果

```
$ cargo build --example novel_crawler
   Compiling wisp v0.1.0 (/home/weng/wisp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.61s

$ cargo test --workspace --lib --bins
   ...
test result: ok. 291 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.82s
```

## 提交

```
575473d response: 新增 enqueue_links/enqueue_links_with 批量链接提取助手
```

## 关切点

1. **brief 测试用例 HTML bug 修复**：`test_enqueue_links_falls_back_to_next_selector_when_first_empty` 原始 HTML 与 selector 语义不一致，已修正 HTML（让 `.list2` 作为 div 容器内嵌 `<ul>`），保留 selector 不变（因其来自示例 novel_crawler.rs 的真实选择器列表）。
2. **detail handler 不使用 `enqueue_links_with`**：按 brief Step 8 "结论" 文档化的设计权衡，detail handler 保留显式循环。原因：detail handler 需要从两处不同 selector 提取（作者 + 章节列表），调用 `enqueue_links_with` 会触发第二次 `parse()` 而 panic。`enqueue_links_with` 的适用边界是「单 selector 查询 + meta 注入」场景。
