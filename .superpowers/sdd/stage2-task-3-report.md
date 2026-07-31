# Stage 2 Task 3 Report: Node 重构为 Arc<Document> + node_id

## Status: DONE_WITH_CONCERNS

所有 parser 相关测试通过（34 lib + 5 adaptive + 7 difflib）。一个关于 Send/Sync 的预防性修复实际未达成目标，详见下文。

## What was implemented

### 1. OnceLock 预防性修改（document.rs）

按 brief 要求把 `std::cell::OnceCell` 改为 `std::sync::OnceLock`：
- `use std::cell::OnceCell;` → `use std::sync::OnceLock;`
- `sxd: OnceCell<Package>` → `sxd: OnceLock<Package>`
- `sxd: OnceCell::new()` → `sxd: OnceLock::new()`
- `get_or_init` API 调用不变

### 2. Cargo.toml 新增 ego-tree 依赖

brief 写的 `use scraper::node::NodeId;` 在 scraper 0.23 中**不存在**。`NodeId` 实际定义在 `ego_tree::NodeId`，是 scraper 的传递依赖。Rust 2021 不允许直接 use 传递依赖的类型，因此新增 `ego-tree = "0.10"` 到 `Cargo.toml`（与 scraper 0.23.1 内部使用的 ego-tree 版本一致）。

### 3. Node struct 重写（mod.rs lines 13-233）

旧 struct：
```rust
pub struct Node {
    inner: Html,
    element_html: Option<String>,
}
```

新 struct：
```rust
pub struct Node {
    doc: Arc<Document>,
    node_id: NodeId,
}
```

新增导入：
```rust
use scraper::{Html, Selector as CssSelector, ElementRef};
use std::sync::Arc;
use ego_tree::NodeId;
use document::Document;
```

### 4. 方法实现细节

- **from_element_ref / element_ref**：私有辅助方法。`element_ref()` 通过 `self.doc.html.tree.get(self.node_id)` 获取 `NodeRef`，再用 `ElementRef::wrap` 转 `Option<ElementRef>`。返回类型显式标注 `Option<ElementRef<'_>>` 避免生命周期隐藏警告。
- **from_html**：用 `Document::from_html` 创建 `Arc<Document>`，node_id 取 `root_element().id()`。
- **from_fragment**：**偏离 brief**。brief 让 from_fragment 直接调用 from_html，但这样 node_id 会指向 `<html>` 根元素而非片段首个元素，导致 `test_attr` / `test_attrs` / `test_html` / `test_outer_html` / `test_generate_selector` / `test_generate_xpath` 全部失败（attr() 返回 None，attrs() 返回空，等等）。修复方案：解析为完整文档后用 `body > *` 选择器定位到 body 下第一个元素，取其 NodeId。这保留了旧 from_fragment 的语义（返回代表片段首个元素的 Node）。若无匹配则回退到 root_element。
- **select / select_one / select_all**：用 `self.doc.html.select(&selector)` 搜索整个文档（不是 scope 到当前节点，这是 brief 接受的行为变化）。每个结果 ElementRef 通过 `from_element_ref(self.doc.clone(), el)` 包装为 Node，共享同一 Arc<Document>。
- **text / html / outer_html / attr / attrs / tag**：通过 `element_ref()` 获取 ElementRef，再调用 scraper 的 `text()` / `inner_html()` / `html()` / `value().attr()` / `value().attrs()` / `value().name()`。新增了 `tag()` 公开方法。
- **children**：用 `element.child_elements()`（scraper 0.23 ElementRef 内置方法，等价于 `children().filter_map(ElementRef::wrap)`，比 brief 的写法更简洁且避免了 brief 中 `.filter(|e| e.value().is_element())` 的编译错误——`ElementRef::value()` 返回 `&Element`，而 `Element` 没有 `is_element()` 方法）。
- **parent / next_sibling / prev_sibling / matches**：临时返回 `None` / `false`（Task 4 真实实现）。
- **first_child / last_child**：基于 `children().first().cloned()` / `last().cloned()`。
- **xpath**：保留 `xpath_to_css` 快速路径（Task 5/6 接入 sxd-xpath）。
- **contains_text / generate_selector / generate_xpath / text_clean / text_regex / inner / css_adaptive**：签名不变，内部用 `self.text()` / `self.doc.html` 等。

### 5. 借用检查器注意点

`from_element_ref(doc: Arc<Document>, el: ElementRef)` 看似有"move doc while el borrows from doc"的问题，但实际调用点是 `Node::from_element_ref(self.doc.clone(), el)`——`self.doc.clone()` 创建新 Arc，`el` 借用的是原 `self.doc`，move 的是 clone，无冲突。在 `from_fragment` 中无法用这个模式（因为 `doc` 是局部变量且 `el` 借用 `doc.html`），所以手动提取 `let id = el.id(); Self { doc, node_id: id }`，依赖 NLL 在 `el.id()` 后释放借用。

## cargo check 输出（关键行）

```
warning: `wisp` (lib) generated 4 warnings (run `cargo fix --lib -p wisp --tests` to apply 1 suggestion)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.98s
```

4 个 warning 全部是预先存在的（`unused_imports`/`unused_variables`/`dead_code` 在 page/scraper/challenge 模块），与本次修改无关。parser/mod.rs 和 parser/document.rs 零 warning。

## 测试输出

### `cargo test --lib`
```
test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 0 measured out; finished in 0.01s
```

### `cargo test --test adaptive_test`
```
test test_css_adaptive_returns_none_when_no_snapshot_and_css_fails ... ok
test test_relocate_returns_none_when_no_match ... ok
test test_relocate_finds_best_match_among_candidates ... ok
test test_capture_then_relocate_after_class_change ... ok
test test_css_adaptive_falls_back_to_snapshot ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 measured out; finished in 0.01s
```

### `cargo test --test difflib_test`
```
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 measured out; finished in 0.00s
```

### `cargo test`（全量）
- checkpoint_test: 4 passed
- crawl_concurrency_test: 0 passed + 1 ignored（需 httpbin 网络）
- integration.rs: 4 passed, **2 failed**（`test_screenshot_creates_file` 和 `test_navigator_webdriver_is_null`，错误分别是 "Not attached to an active page" 和 "Timeout CDP: Page.enable"——这是 CDP 浏览器自动化测试，需要真实 Chrome 实例，**与本次 parser 重构无关**，在基线 commit 2f3285e 上同样会失败）

## scraper 0.23 API 调整说明

1. **`NodeId` 来源**：brief 写 `use scraper::node::NodeId;`，实际 scraper 0.23 不导出 NodeId。改用 `use ego_tree::NodeId;`，并在 Cargo.toml 新增 `ego-tree = "0.10"` 直接依赖。
2. **`ElementRef::wrap` 返回 `Option<ElementRef>`**：与 brief 一致，0.23 确实是 `Option`（不是直接返回 ElementRef）。
3. **`element.id()`**：通过 `ElementRef` 的 `Deref` to `NodeRef` 调用 `NodeRef::id()` 返回 `NodeId`（Copy 类型）。brief 写的 `el.id()` 正确。
4. **`element.children()` + `ElementRef::wrap`**：brief 写的 `.filter(|e| e.value().is_element())` 在 0.23 上编译失败（`ElementRef::value()` 返回 `&Element`，`Element` 没有 `is_element()` 方法；`is_element()` 是 `scraper::Node` enum 的方法）。改用 `element.child_elements()`（ElementRef 内置方法，已封装 children + filter_map wrap），功能等价。
5. **`Html.tree` 是 pub 字段**：`self.doc.html.tree.get(node_id)` 返回 `Option<NodeRef<Node>>`，可直接调用。

## Send/Sync 预防性修复实际未达成目标（重要 concern）

brief 声称"把 OnceCell 改为 OnceLock 后 Document 就是 Sync 的，Arc<Document> 可以跨线程共享"。**这个声称不成立**。

通过临时编译期断言验证（已删除）：
```rust
fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}
assert_send::<Arc<Document>>();  // 编译失败
assert_sync::<Arc<Document>>();  // 编译失败
```

编译错误：
```
error[E0277]: `*mut sxd_document::raw::Root` cannot be shared between threads safely
   = help: within `Package`, the trait `Sync` is not implemented for `*mut sxd_document::raw::Root`
   = note: required for `OnceLock<Package>` to implement `Sync`
   = note: required for `Arc<document::Document>` to implement `std::marker::Send`

error[E0277]: `Cell<*mut u8>` cannot be shared between threads safely
   = help: within `Package`, the trait `Sync` is not implemented for `Cell<*mut u8>`
```

**根因**：`sxd_document::Package` 内部 `Connections { root: *mut Root }` 和 `Storage` 中的 `Cell<*mut u8>` / `Cell<*const u8>` 等字段使其 `!Send + !Sync`。`OnceLock<T>: Sync` 要求 `T: Send + Sync`，所以 `OnceLock<Package>` 仍然 `!Sync`，`Document` 仍然 `!Sync`，`Arc<Document>: !Send + !Sync`。

OnceCell → OnceLock 的修改本身没错（是必要条件），但**不充分**。要真正实现 `Arc<Document>: Send + Sync`，需要后续 task 处理 sxd-package 的线程安全，可选方案：
1. 用 `Mutex<Package>` 替代 `OnceLock<Package>`（Mutex<T>: Sync 只需 T: Send）
2. 用 `parking_lot::Mutex<Package>`
3. 不在 Document 中存储 Package，改为每次按需构建（性能差）
4. 自定义 newtype wrapper 配合 `unsafe impl Send + Sync`（需谨慎审计 sxd-document 的线程安全语义）

**当前影响**：spider 并发代码若把 `Node` move 到 async block 跨 await 持有，会触发编译错误。但当前测试套件无此场景，所有测试通过。这个限制留给后续 task 处理（可能是 Task 4 或独立的线程安全 task）。

## Files changed

1. `f:\project\wisp\Cargo.toml` — 新增 `ego-tree = "0.10"` 依赖
2. `f:\project\wisp\src\parser\document.rs` — OnceCell → OnceLock（3 处：import / 字段类型 / 构造）
3. `f:\project\wisp\src\parser\mod.rs` — 重写 Node struct + impl（lines 13-233），保留 NodeList / xpath_to_css / helpers / tests 不动

## Self-review findings

1. **select() 语义变化**：旧 select() 在每个结果的 fragment 上重新 parse，新 select() 共享 Document。对于当前测试（只在 root Node 上调 select）无影响。但若用户在子 Node 上调 select()，旧代码 scope 到子树，新代码搜索整个文档——这是 brief 接受的行为变化，Task 4 应考虑用 `element_ref().select()` 实现 scoped 查询。
2. **from_fragment 偏离 brief**：如上所述，必须偏离才能让测试通过。已加注释说明。
3. **children() 用 child_elements()**：比 brief 的写法更简洁且避免了 brief 中的编译错误。
4. **Send/Sync 未达成**：见上节，是后续 task 的工作。
5. **inner() 返回 &Html**：`&self.doc.html` 借用 `self.doc`（Arc），生命周期绑定到 `&self`，符合预期。
6. **matches() 参数改名 `_css`**：避免 unused variable warning。
7. **保留的方法签名全部不变**：from_html/from_fragment/select/select_one/select_all/text/attr/attrs/html/outer_html/tag(新增)/css_adaptive/inner/parent/children/next_sibling/prev_sibling/first_child/last_child/matches/xpath/contains_text/generate_selector/generate_xpath/text_clean/text_regex。

## Commit SHA

650e8850f1f5a66cfdb41ec169ca1083699c22dd — `refactor: Node 重构为 Arc<Document> + node_id 共享所有权`

---

# Review Findings 修复（2026-07-21）

## 修复的 Findings

### Important 2: from_fragment 对表格类片段行为退化

**问题**：Task 3 重构后 `Node::from_fragment` 改用 `Document::from_html`（`Html::parse_document`），Important 2 描述 html5ever 会把 `<td>/<tr>` 等表格元素"强制包裹 `<table><tbody><tr>`"，导致 `tag()` 返回 "table"。

**实际行为调查**（通过调试测试验证 scraper 0.23.1）：
- `Html::parse_document("<td>cell</td>")` → html() = `<html><head></head><body>cell</body></html>`，**`<td>` 标签被完全丢弃**（不是包裹 `<table>`），只保留 "cell" 文本。
- `Html::parse_fragment("<td>cell</td>")` → html() = `<html>cell</html>`，**`<td>` 同样被丢弃**。
- 根因：HTML5 规范 "in body" insertion mode 下遇到 `<td>` 时，若 stack 上无 table/tbody/tr 等上下文，会 "ignore the token"（丢弃标签）。`parse_fragment` 用 `<body>` context，`parse_document` 初始 stack 也是 `[html, body]`，都没有 table 上下文。
- **Important 2 原描述（"包裹 `<table>`"）不准确**，实际行为是丢弃标签。

**修复方案**（混合策略，偏离 brief 的"统一用 parse_fragment"）：
1. 提取片段开头的标签名（`<td>...` → "td"）
2. 检测是否是表格元素（`td/tr/th/thead/tbody/tfoot/caption/colgroup/col`）
3. **表格元素片段**：包裹 `<table>{html}</table>` 后用 `Document::from_html`（`parse_document`）解析。html5ever 在 `<table>` context 下会规范化为 `<table><tbody><tr><td>cell</td></tr></tbody></table>`，**保留 `<td>` 标签**。然后用原始标签名作为 CSS 选择器深入找到实际的片段元素。
4. **普通元素片段**：用 `Document::from_fragment`（`parse_fragment`）解析，保留片段语义（不创建 `<html><head><body>` 结构）。`parse_fragment` 创建 `<html>` root_element，片段内容直接在其下，用 `html > *` 选择器定位。

**新增方法**：`Document::from_fragment`（用 `Html::parse_fragment`），供普通元素片段路径使用。

**验证**：
- `Node::from_fragment("<td>cell</td>").tag()` → "td" ✓
- `Node::from_fragment("<td>cell</td>").text()` → "cell" ✓
- `Node::from_fragment("<td>cell</td>").outer_html()` → 包含 `<td>cell</td>` ✓
- 普通元素片段（`<a>/<div>/...`）行为与旧实现一致 ✓

### Minor 1: from_fragment 对裸文本/注释片段行为退化

**修复**：在 `Node::from_fragment` 的文档注释中标注："裸文本/注释片段在 `html > *` 不匹配元素时会回退到 root_element（此时 `tag()` 返回 `<html>`，可能不是用户期望的结果）"。

### Minor 3: select() scoped→global 语义变化未在代码中标注

**修复**：在 `Node::select()` 的文档注释中加说明："注意：当前实现搜索整个文档（`self.doc.html.select`），不 scope 到当前 Node 的子树。这是 Task 3 重构带来的语义变化（旧实现 scope 到子树）。Task 4 计划用 `element_ref().select()` 实现 scoped 查询。"

## 测试结果

### `cargo test --lib`（含新回归测试）
```
test parser::tests::test_from_fragment_table_element ... ok
... (其余 34 个测试全部 ok)
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 0 measured out; finished in 0.01s
```

### `cargo test --test adaptive_test`
```
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 measured out; finished in 0.01s
```

### `cargo test --test difflib_test`
```
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 measured out; finished in 0.00s
```

## 新增回归测试

- **名称**：`parser::tests::test_from_fragment_table_element`
- **验证**：`Node::from_fragment("<td>cell</td>")` 的 `tag()` 返回 "td"、`text()` 含 "cell"、`outer_html()` 含 `<td>cell</td>`
- **结果**：通过

## Files changed

1. `f:\project\wisp\src\parser\document.rs` — 新增 `Document::from_fragment` 方法（用 `Html::parse_fragment`），并给 `from_html` 加文档注释说明 HTML5 结构规则
2. `f:\project\wisp\src\parser\mod.rs` — 重写 `Node::from_fragment`（混合方案：表格元素包裹 `<table>` + 普通元素用 `parse_fragment`）；`select()` 加 scoped→global 语义变化文档注释；新增 `test_from_fragment_table_element` 回归测试

## Commit SHA

cfd88b5 — `fix: from_fragment 表格片段包裹 table 保留语义 + select 文档注释`

## 备注

- brief 假设 `Html::parse_fragment` 会保留 `<td>` 标签，但实际 scraper 0.23.1 的 `parse_fragment` 用 `<body>` context，HTML5 规范下 `<td>` 在 `<body>` 中不合法会被丢弃。因此采用了"表格元素包裹 `<table>` + `parse_document`"的混合方案，而非 brief 建议的"统一用 `parse_fragment`"。
- `Document::from_fragment` 方法仍然新增（brief 要求），供普通元素片段路径使用。
- 所有原有测试（34 lib）+ 1 新回归测试 = 35 lib 全部通过；adaptive + difflib 不受影响。
