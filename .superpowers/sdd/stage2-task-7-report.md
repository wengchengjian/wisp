# Stage 2 Task 7 报告：ElementSnapshot::capture 升级用 Node 导航 API

- **Status:** DONE
- **文件：** `src/parser/adaptive.rs`
- **Brief：** `.superpowers/sdd/stage2-task-7-brief.md`

## 实现内容

### Step 2: 重写 `capture` 函数（`src/parser/adaptive.rs` 行 31-98）

将旧的 `capture`（用 `Html::parse_document` 重复解析 `outer_html` 拿 `ElementRef`）替换为直接使用 Node 导航 API 的实现：
- `node.tag()` / `node.attrs()` / `node.text()` —— 取基本信息
- `node.ancestors()` —— 遍历祖先，每级生成 `"tag"` 或 `"tag.firstclass"`，`.rev()` 使根在前
- `node.parent()` —— 取父节点上下文（`parent_tag` / `parent_attrs`）
- `parent.children()` —— 取兄弟节点 tag 列表（`sibling_tags`）
- `node.outer_html()` + `parent.children().position()` —— 计算 `position_in_parent`（用 outer_html 比较身份，避免相同 tag+text 的兄弟节点误匹配）

`text_preview` 截断逻辑：长度 > 200 时 `chars().take(200).collect()`，否则原样保留。

### Step 3: 删除旧函数

- 删除 `capture_from_element_ref`（原行 52-121）
- 删除 `capture_from_node_only`（原行 124-138）

净变化：`+47 / -87`。

### Step 4: 保留 4 个 helper 函数

未删除（仍被 `similarity()` 使用，line 192/225/231/237）：
- `node_tag_name`（line 301）
- `ancestor_path_of`（line 314）
- `sibling_tags_of`（line 345）
- `parent_attrs_of`（line 366）

### Step 5: 保留 import

全部 import 保留（helper 仍使用）：
- `use scraper::{Html, ElementRef};`（line 8）
- `use scraper::node::Node as ScraperNode;`（line 9）
- `use super::Node;`（line 10，等价于 brief 中的 `use crate::parser::Node;`，未重复添加）

### 关于 brief 中 `use crate::parser::Node;` 的说明

brief 的 Step 2 代码块顶部含 `use crate::parser::Node;`，但文件已有 `use super::Node;`（等价、同模块路径），按 rust-best-practices「最小改动」原则不重复添加。

## 测试结果

### Step 6: `cargo check`

```
exit code 0
Finished `dev` profile in 3.27s
```
（4 个 warning 均为既有问题，位于 `browser/mod.rs`、`scraper/mod.rs`、`page/mod.rs`、`challenge/mod.rs`，与本次改动无关。）

### Step 7: `cargo test --test adaptive_test`

```
running 5 tests
test test_css_adaptive_returns_none_when_no_snapshot_and_css_fails ... ok
test test_relocate_returns_none_when_no_match ... ok
test test_relocate_finds_best_match_among_candidates ... ok
test test_capture_then_relocate_after_class_change ... ok
test test_css_adaptive_falls_back_to_snapshot ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```
**5 passed**（期望一致：capture + relocate 行为不变）。

### Step 8: 全套测试

| 命令 | 结果 |
|------|------|
| `cargo test --lib` | **35 passed; 0 failed** |
| `cargo test --test dom_navigation_test` | **9 passed; 0 failed** |
| `cargo test --test xpath_test` | **9 passed; 0 failed** |
| `cargo test --test crawl_checkpoint_test` | **4 passed; 0 failed** |
| `cargo test --test difflib_test` | **7 passed; 0 failed** |

合计 **64 passed; 0 failed**（加上 Step 7 的 5 个，共 69 个测试全部通过）。最终打印 `ALL_PASSED`，exit code 0。

## 提交

- Hash: `506c069`
- Message: `refactor: ElementSnapshot::capture 升级用 Node 导航 API（消除重复解析）`
- 范围: `1 file changed, 47 insertions(+), 87 deletions(-)`（仅 `src/parser/adaptive.rs`）

## Concerns

无。

## Self-Review Notes

1. **行为等价性**：新 `capture` 与旧 `capture_from_element_ref` 语义一致 —— ancestor_path 用 `first_class` 拼接、sibling_tags 取父节点元素子节点、position 用元素子节点索引。旧实现 position 用 `ElementRef::wrap(c) == Some(*el)` 比较，新实现改用 `outer_html() == target_html` 比较（brief 明确要求，注释说明更准确，避免相同 tag+text 兄弟节点误匹配）。
2. **text_preview 截断**：旧实现无条件 `chars().take(200).collect()`；新实现按 brief 要求，长度 ≤ 200 时跳过 collect 直接用原字符串（微小性能优化，行为等价）。
3. **消除重复解析**：旧路径每次 `capture` 调用 `Html::parse_document` 一次（capture 内）+ 4 个 helper 各再解析一次（`similarity` 内）= 5 次解析；新 `capture` 0 次解析。`similarity` 路径的 4 次解析不在本 task 范围（brief Step 4 明确）。
4. **未触碰范围外文件**：未改 4 个 helper、未改 import、未改其他文件。
5. **Git CRLF 警告**：`warning: LF will be replaced by CRLF` 是 Windows 仓库既有行为，不影响内容。
