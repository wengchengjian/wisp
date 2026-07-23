# Task 6 报告：修复 CSS 选择器解析失败静默回退到 `*`

## Status
✅ COMPLETE

## Commit
`51c0628` — `fix(parser): 非法 CSS 选择器返回空而非回退到 *`（分支 `fix/code-review-2026-07-23`）

## 修改文件
- `src/parser/mod.rs`（+29 / -3）
  - `Node::select`（L104-116）：`unwrap_or_else(|_| CssSelector::parse("*").unwrap())` → `let Ok(selector) = CssSelector::parse(css) else { return NodeList { nodes: Vec::new() }; };`
  - `Node::from_fragment` 表格分支（L80-87）：`unwrap_or_else(|_| CssSelector::parse("*").unwrap())` → `match` 标签名非法时回退到 `root_element`
  - `#[cfg(test)] mod tests`：新增 `select_invalid_selector_returns_empty_not_all` 与 `select_valid_selector_still_works`

## 测试摘要
- TDD Step 2（修复前）：`select_invalid_selector_returns_empty_not_all` FAIL，实际返回 4 个元素（html/body/p/p 被 `*` 匹配），符合预期失败
- `cargo build`：通过（仅原有 7 个无关 warning）
- `cargo test --lib parser`：22 passed / 0 failed（含 2 个新增 + 现有 20 个，包括 `test_from_fragment_table_element` 表格回归）
- `cargo test --lib`：199 passed / 0 failed / 0 ignored（全量无回归）

## 实现细节与决策
- **`select` 用 let-else**：edition 2021 + Rust 1.65+ 支持，与 brief Step 3 一致；返回类型仍为 `NodeList`，公开 API 向后兼容，仅改失败语义
- **`from_fragment` 表格分支 match arm 取 `root_id` 后再 move `doc`**：Rust 借用检查器不允许 `return Self { doc, node_id: doc.html.root_element().id() }` 这种 move 后 borrow 的写法，故先在 arm 内取 `root_id` 再 return。这是 brief 方案的必要调整（brief 代码会编译失败）
- **`inner_tag` 提取自片段开头 `<td...` 的字母数字**，正常情况必为合法标签名（td/tr/th/...），但若用户传入 `<>xxx</>` 等畸形片段，`CssSelector::parse("")` 会失败，此时回退 root_element 是合理退化
- 注释中文，无可恢复路径上的 unwrap/expect

## 顾虑
- 无架构决策，未触发 BLOCKED
- `from_fragment` 表格分支的非法回退路径在实际中几乎不会被触发（inner_tag 由 `take_while(is_alphanumeric)` 提取，必为非空字母数字串），但保持防御性一致
- 未删除/修改 `select_all`、`NodeList::select`（它们都委托 `Node::select`，自动继承新行为）

## 报告路径
`/home/weng/wisp/.superpowers/sdd/task-6-report.md`
