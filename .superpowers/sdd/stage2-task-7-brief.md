# Task 7: ElementSnapshot::capture 升级用 Node 导航 API

**Files:**
- Modify: `src/parser/adaptive.rs`

**目标：** 阶段 1 的 capture 用 scraper::ElementRef 临时拿上下文（每次 similarity 调用重复解析 outer_html 4 次）。阶段 2 Node 重构后，capture 改用 Node 的 ancestors()/parent()/children() 导航 API，消除 capture 路径的重复解析。

**重要：** 4 个 helper 函数（node_tag_name / ancestor_path_of / sibling_tags_of / parent_attrs_of）也被 `similarity()` 函数使用，**保留不删**（它们的重写不在本 task 范围）。只删 `capture_from_element_ref` 和 `capture_from_node_only`（仅 capture 使用）。

## Step 1: 读取当前 ElementSnapshot::capture 实现

读取 `src/parser/adaptive.rs` 的 `capture` 函数（line 36-49）、`capture_from_element_ref`（line 52-121）、`capture_from_node_only`（line 124-138）。

## Step 2: 重写 capture 用 Node 导航 API

在 `src/parser/adaptive.rs` 中替换 `capture` 函数（删除 capture_from_element_ref 和 capture_from_node_only）：

```rust
use crate::parser::Node;

impl ElementSnapshot {
    /// 从 Node 捕获快照（用 Node 导航 API，不再重复解析 outer_html）。
    pub fn capture(node: &Node) -> Self {
        let tag = node.tag();
        let attrs = node.attrs();
        let text_preview = node.text();
        let text_preview = if text_preview.len() > 200 {
            text_preview.chars().take(200).collect()
        } else {
            text_preview
        };

        // ancestor_path: 从父节点到根，每级 "tag" 或 "tag.firstclass"，最后 rev() 使根在前
        let ancestor_path: Vec<String> = node.ancestors()
            .filter_map(|n| {
                let t = n.tag();
                if t.is_empty() {
                    return None;
                }
                let class = n.attr("class").unwrap_or_default();
                if class.is_empty() {
                    Some(t)
                } else {
                    let first_class: String = class.split_whitespace().next().unwrap_or("").to_string();
                    if first_class.is_empty() {
                        Some(t)
                    } else {
                        Some(format!("{}.{}", t, first_class))
                    }
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // parent context
        let parent_node = node.parent();
        let parent_tag = parent_node.as_ref().map(|p| p.tag()).unwrap_or_default();
        let parent_attrs = parent_node.as_ref().map(|p| p.attrs()).unwrap_or_default();

        // sibling_tags: 父节点的所有元素子节点的 tag 列表
        let sibling_tags: Vec<String> = parent_node.as_ref()
            .map(|p| p.children().iter().map(|c| c.tag()).collect())
            .unwrap_or_default();

        // position_in_parent: 当前节点在父节点子元素中的索引
        // 用 outer_html 比较身份（比 tag+text 更准确，避免相同 tag+text 的兄弟节点误匹配）
        let position_in_parent = match &parent_node {
            Some(p) => {
                let target_html = node.outer_html();
                p.children().iter()
                    .position(|c| c.outer_html() == target_html)
                    .unwrap_or(0)
            }
            None => 0,
        };

        Self {
            tag,
            attrs,
            text_preview,
            ancestor_path,
            sibling_tags,
            position_in_parent,
            parent_tag,
            parent_attrs,
        }
    }
}
```

## Step 3: 删除 capture_from_element_ref 和 capture_from_node_only

删除 `capture_from_element_ref`（line 52-121）和 `capture_from_node_only`（line 124-138）这两个函数。它们只被旧 capture 使用。

## Step 4: 保留 4 个 helper 函数

**不要删除** `node_tag_name` / `ancestor_path_of` / `sibling_tags_of` / `parent_attrs_of`（line 341-419）。它们被 `similarity()` 函数使用（line 192, 225, 231, 237）。它们的重写不在本 task 范围。

## Step 5: 清理不再需要的 import

删除 capture_from_element_ref 后，可能不再需要 `use scraper::{Html, ElementRef};` 和 `use scraper::node::Node as ScraperNode;`。检查 4 个 helper 是否还用这些 import：
- `node_tag_name` 用 `Html::parse_fragment` —— 保留 `Html`
- `ancestor_path_of` / `sibling_tags_of` / `parent_attrs_of` 用 `Html::parse_document` 和 `ScraperNode::Element` —— 保留 `Html` 和 `ScraperNode`
- `ElementRef` 在 `sibling_tags_of` / `parent_attrs_of` 中用 —— 保留 `ElementRef`

所以 import 全部保留。

## Step 6: 运行 cargo check 验证编译

Run: `cargo check`
Expected: 编译通过

## Step 7: 运行 adaptive 测试

Run: `cargo test --test adaptive_test`
Expected: 5 passed（capture + relocate 行为不变）

## Step 8: 运行全部测试确保未破坏

Run: `cargo test --lib && cargo test --test dom_navigation_test && cargo test --test xpath_test && cargo test --test crawl_checkpoint_test && cargo test --test difflib_test`
Expected: 全部通过

## Step 9: 提交

```bash
git add src/parser/adaptive.rs
git commit -m "refactor: ElementSnapshot::capture 升级用 Node 导航 API（消除重复解析）"
```
