# Task 5: sxd-xpath 完整查询集成

**Files:**
- Create: `src/parser/xpath.rs`
- Modify: `src/parser/mod.rs`（声明子模块 + 接入 xpath 方法）

## Step 1: 创建 src/parser/xpath.rs

```rust
//! sxd-xpath 完整查询集成。
//!
//! 快速路径（xpath_to_css）覆盖 80% 常见 XPath，慢路径用 sxd-xpath 执行完整 XPath 1.0。
//! 结果回查 scraper 树：用 tag + 属性 + 路径定位 sxd 节点对应的 scraper 节点。

use sxd_document::dom;
use sxd_xpath::{Factory, Value};
use crate::error::{WispError, Result};
use super::{Document, Node, NodeList};
use std::sync::Arc;

/// 执行完整 sxd-xpath 查询。
///
/// 1. 懒加载 sxd-document Package
/// 2. 定位当前节点在 sxd 树中的对应节点
/// 3. 执行 xpath 查询
/// 4. 结果回查 scraper 树
pub fn xpath_full(node: &Node, expr: &str) -> Result<NodeList> {
    let document = node.doc.sxd_package();
    let doc = document.as_document();

    // 定位当前节点在 sxd 树中的对应节点（用 tag + 路径启发式定位）
    let context_element = locate_in_sxd(doc, node).unwrap_or(doc.root());

    // 解析并执行 xpath
    let factory = Factory::new();
    let xpath = factory.build(expr)
        .map_err(|e| WispError::ParseError(format!("xpath parse: {e}")))?
        .ok_or_else(|| WispError::ParseError(format!("xpath empty: {expr}")))?;

    let value = xpath.evaluate(doc, context_element)
        .map_err(|e| WispError::ParseError(format!("xpath evaluate: {e}")))?;

    // 结果转回 NodeList
    match value {
        Value::Nodeset(ns) => {
            let nodes: Vec<Node> = ns.iter()
                .filter_map(|n| find_in_scraper(&node.doc, n))
                .collect();
            Ok(NodeList { nodes })
        }
        _ => Ok(NodeList { nodes: Vec::new() }),
    }
}

/// 在 sxd 树中定位 scraper 节点的对应节点。
///
/// 启发式：用当前节点的 tag + 属性匹配 sxd 树中的第一个同名同属性节点。
/// 找不到则返回 None（调用方回退到 doc.root()）。
fn locate_in_sxd<'d>(doc: sxd_document::dom::Document<'d>, node: &Node) -> Option<dom::Element<'d>> {
    let target_tag = node.tag();
    if target_tag.is_empty() {
        return None;
    }
    // 简化：找第一个同名元素。精确匹配需用路径，stage 2 接受此简化。
    doc.descendants()
        .filter_map(|n| match n {
            dom::ChildOfElement::Element(e) => Some(e),
            _ => None,
        })
        .find(|e| e.name() == target_tag.as_str())
}

/// 在 scraper 树中找到 sxd 节点的对应节点。
///
/// 用 tag + text 内容 + 属性启发式匹配。找不到则跳过（不 panic）。
fn find_in_scraper(doc: &Arc<Document>, sxd_node: &dom::Element) -> Option<Node> {
    let tag = sxd_node.name().local_part();
    let attrs: Vec<(String, String)> = sxd_node.attributes()
        .iter()
        .map(|a| (a.name().local_part().to_string(), a.value().to_string()))
        .collect();

    // 在 scraper 树中找第一个 tag + 属性匹配的元素
    let selector_str = if attrs.is_empty() {
        tag.to_string()
    } else {
        // 用第一个属性构造选择器
        let (k, v) = &attrs[0];
        format!("{}[{}='{}']", tag, k, v)
    };

    let selector = scraper::Selector::parse(&selector_str).ok()?;
    doc.html.select(&selector)
        .next()
        .map(|el| Node::from_element_ref(doc.clone(), el))
}
```

**重要 API 注意事项**：sxd-document 0.3.2 和 sxd-xpath 0.4.2 的 API 可能与上面代码不完全一致。已知可能的问题：
- `dom::ChildOfElement` 枚举可能不存在 — sxd-document 0.3.2 的 `Document::descendants()` 返回的是 `ChildOfElement` 枚举或直接 `Element`，需查实际 API
- `doc.descendants()` 可能返回不同类型
- `sxd_node.name()` 返回 `QName`，`.local_part()` 返回 `&str`
- `Value::Nodeset` 的 `ns.iter()` 返回 `&dom::Element` 或 `&Value`
- `xpath.evaluate(doc, context_element)` 的第二个参数类型可能是 `&dyn sxd_xpath::context::Item`

**如果编译失败**，根据 sxd-document 0.3.2 和 sxd-xpath 0.4.2 的实际 API 调整。可以用 `cargo doc -p sxd-document --open` 或 `cargo doc -p sxd-xpath --open` 查看文档。不要猜——查文档确认。

## Step 2: 在 src/parser/mod.rs 声明 xpath 子模块

在 `pub mod document;` 之后追加：

```rust
pub mod xpath;
```

当前 src/parser/mod.rs 的模块声明区是：
```rust
pub mod difflib;
pub mod document;
pub mod adaptive;
pub mod generate;
```

改为：
```rust
pub mod difflib;
pub mod document;
pub mod xpath;
pub mod adaptive;
pub mod generate;
```

## Step 3: 修改 Node::xpath 接入 xpath_full

找到当前的 xpath 方法（Task 3 保留的临时实现，快速路径 + 空慢路径），替换为：

```rust
    /// Select all elements matching an XPath expression.
    ///
    /// 支持完整 XPath 1.0。简单表达式走 xpath_to_css 快速路径，
    /// 复杂表达式走 sxd-xpath 完整查询。
    pub fn xpath(&self, expr: &str) -> NodeList {
        // 快速路径：简单 XPath 转 CSS（覆盖 80% 常见用法）
        if let Some(css) = xpath_to_css(expr) {
            return self.select(&css);
        }
        // 慢路径：完整 sxd-xpath 查询
        match xpath::xpath_full(self, expr) {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!("xpath 查询失败 '{}': {}", expr, e);
                NodeList { nodes: Vec::new() }
            }
        }
    }
```

## Step 4: 运行 cargo check 验证编译

Run: `cargo check`
Expected: 编译通过。如果 sxd API 不匹配，根据实际 API 修复。

## Step 5: 提交

```bash
git add src/parser/xpath.rs src/parser/mod.rs
git commit -m "feat: sxd-xpath 完整查询集成（懒加载 + 结果回查 scraper 树）"
```
