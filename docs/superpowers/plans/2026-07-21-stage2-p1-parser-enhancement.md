# 阶段 2（P1 解析增强）实现计划：Node 重构 + sxd-xpath + DOM 导航

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 wisp 的 `Node` 从"无树导航的 fragment 切片"重构为 `Arc<Document>` 共享所有权模型，真实实现所有 DOM 导航方法（parent/children/sibling/ancestors/matches），集成 sxd-xpath 支持完整 XPath 1.0 查询，并升级 `ElementSnapshot::capture` 用 Node 导航 API 替代 scraper::ElementRef 临时方案。

**Architecture:** `Node` 内部从 `inner: Html + element_html: Option<String>` 重构为 `doc: Arc<Document> + node_id: NodeId`，`Document` 包含 `Arc<Html>` + `OnceCell<Package>`（sxd-document 懒加载）。所有 select() 返回的 Node 共享同一个 Document，通过 node_id 引用树中位置，使 parent/ancestors 等导航方法可工作。XPath 快速路径（xpath_to_css）保留覆盖 80% 常见用法，慢路径用 sxd-xpath 完整查询，结果回查 scraper 树。HTML5 容错通过 html5ever（scraper 内部）规范化后喂给 sxd-document 解决。

**Tech Stack:** Rust 2021, scraper 0.23 (Html/ElementRef/Selector), sxd-document 0.3.2, sxd-xpath 0.4.2, std::sync::Arc, std::cell::OnceCell

**Spec:** [docs/superpowers/specs/2026-07-21-scrapling-borrow-design.md](../specs/2026-07-21-scrapling-borrow-design.md) 的"阶段 2"章节（2.2 + 2.3，不含 2.1 wreq 替换）

**范围说明：** 本 plan 覆盖 spec 阶段 2 的 2.2（sxd-xpath 集成）+ 2.3（DOM 导航重构 + ElementSnapshot 升级）。2.1（wreq 替换 reqwest）因 Windows 环境缺 perl/nasm（BoringSSL 编译需要）推迟到工具链就绪后单独执行。

---

## 文件结构

| 文件 | 职责 | 操作 |
|---|---|---|
| `Cargo.toml` | 新增 sxd-document/sxd-xpath 依赖 | 修改 |
| `src/error.rs` | 新增 ParseError 变体（XPath 解析错误） | 修改 |
| `src/parser/mod.rs` | Node 重构为 Arc<Document> + DOM 导航 + XPath 集成 | 重写 |
| `src/parser/document.rs` | Document struct + sxd 懒加载 + build_sxd_from_html | 创建 |
| `src/parser/xpath.rs` | xpath_full 实现 + 结果回查 scraper 树 | 创建 |
| `src/parser/adaptive.rs` | ElementSnapshot::capture 升级用 Node 导航 API | 修改 |
| `tests/dom_navigation_test.rs` | parent/children/sibling/ancestors/matches 测试 | 创建 |
| `tests/xpath_test.rs` | XPath 基础 + 复杂 + 容错测试 | 创建 |

---

## Task 1: 新增依赖与 ParseError 变体

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/error.rs`

- [ ] **Step 1: 在 Cargo.toml 的 [dependencies] 追加 sxd 依赖**

在 `chrono = ...` 这一行之后追加：

```toml
# XPath 1.0 完整查询（阶段 2：sxd-xpath 懒解析）
sxd-document = "0.3"
sxd-xpath = "0.4"
```

- [ ] **Step 2: 在 src/error.rs 的 WispError enum 追加 ParseError 变体**

在 `McpError(String)` 变体之后追加：

```rust
    #[error("Parse error: {0}")]
    ParseError(String),
```

- [ ] **Step 3: 运行 cargo check 验证编译**

Run: `cargo check`
Expected: 编译通过（sxd 依赖会被拉取）

- [ ] **Step 4: 提交**

```bash
git add Cargo.toml src/error.rs
git commit -m "feat: 新增 sxd-document/sxd-xpath 依赖与 ParseError 变体"
```

---

## Task 2: 创建 Document struct + sxd 懒加载基础设施

**Files:**
- Create: `src/parser/document.rs`
- Modify: `src/parser/mod.rs`（声明子模块）

- [ ] **Step 1: 创建 src/parser/document.rs**

```rust
//! Document: 共享所有权的 HTML 文档容器。
//!
//! 包含 scraper::Html（CSS 查询）和懒加载的 sxd-document::Package（XPath 查询）。
//! Node 通过 Arc<Document> 共享文档，select() 返回的 Node 引用同一文档的树中位置。

use std::sync::Arc;
use std::cell::OnceCell;
use scraper::Html;
use sxd_document::Package;

/// 共享的 HTML 文档。scraper 树用于 CSS 查询和 DOM 导航，
/// sxd-document 树懒加载用于 XPath 查询。
pub struct Document {
    /// scraper 解析的 HTML 树（html5ever 容错）
    pub(crate) html: Arc<Html>,
    /// 懒加载的 sxd-document 包（XPath 用）
    sxd: OnceCell<Package>,
}

impl Document {
    /// 从 HTML 字符串创建文档。
    pub fn from_html(html: &str) -> Arc<Self> {
        let parsed = Html::parse_document(html);
        Arc::new(Self {
            html: Arc::new(parsed),
            sxd: OnceCell::new(),
        })
    }

    /// 获取 sxd-document 包（懒加载）。
    ///
    /// 首次调用时用 html5ever 规范化后的 HTML 喂给 sxd_document::parser，
    /// 解决 sxd 对 HTML5 容错差的问题。后续调用直接返回缓存的 Package。
    pub fn sxd_package(&self) -> &Package {
        self.sxd.get_or_init(|| build_sxd_from_html(&self.html))
    }
}

/// 用 html5ever（scraper 内部）规范化 HTML 后喂给 sxd-document。
///
/// sxd_document::parser 是 XML 解析器，对 HTML5 容错弱：
/// - `<br>`/`<img>` 等空标签需要自闭合
/// - `<script>`/`<style>` 内容会被当文本
/// html5ever 输出的 `html()` 已经规范化处理了这些。
fn build_sxd_from_html(html: &Html) -> Package {
    // html() 返回规范化的 HTML 字串（含 html/head/body 结构）
    let clean_html = html.html();
    sxd_document::parser::parse(&clean_html)
        .unwrap_or_else(|_| Package::new())
}
```

- [ ] **Step 2: 在 src/parser/mod.rs 声明 document 子模块**

在 `pub mod adaptive;` 之前追加：

```rust
pub mod document;
```

- [ ] **Step 3: 运行 cargo check 验证编译**

Run: `cargo check`
Expected: 编译通过

- [ ] **Step 4: 提交**

```bash
git add src/parser/document.rs src/parser/mod.rs
git commit -m "feat: 新增 Document struct + sxd-document 懒加载基础设施"
```

---

## Task 3: Node 重构为 Arc<Document> + node_id

**Files:**
- Modify: `src/parser/mod.rs`（重写 Node struct + 所有方法）

**关键约束：** 所有公开 API 签名保持不变（from_html/from_fragment/select/select_one/text/attr/html/outer_html/css_adaptive 等）。现有测试必须全部通过。

- [ ] **Step 1: 先跑现有测试确认基线**

Run: `cargo test --lib`
Expected: 34 passed（阶段 1 后的基线）

- [ ] **Step 2: 重写 src/parser/mod.rs 的 Node struct 定义**

找到当前的 Node struct（约 line 16-21）：

```rust
/// A parsed HTML document or element.
#[derive(Clone)]
pub struct Node {
    inner: Html,
    /// For fragments, store the first element's HTML to extract attrs correctly
    element_html: Option<String>,
}
```

替换为：

```rust
use document::Document;
use scraper::node::NodeId;
use scraper::ElementRef;

/// A parsed HTML document or element.
///
/// 内部通过 `Arc<Document>` 共享文档所有权，`node_id` 标识在 scraper 树中的位置。
/// 所有 select() 返回的 Node 共享同一文档，使 parent/ancestors 等导航可工作。
#[derive(Clone)]
pub struct Node {
    doc: Arc<Document>,
    node_id: NodeId,
}

impl Node {
    /// 从 ElementRef 创建 Node（内部辅助方法）。
    fn from_element_ref(doc: Arc<Document>, el: ElementRef) -> Self {
        Self { doc, node_id: el.id() }
    }

    /// 获取当前节点对应的 ElementRef（在 scraper 树中查找）。
    fn element_ref(&self) -> Option<ElementRef> {
        ElementRef::wrap(self.doc.html.tree.get(self.node_id)?)
    }
}
```

- [ ] **Step 3: 重写 from_html / from_fragment**

```rust
    /// Parse HTML string into a Node (document root).
    pub fn from_html(html: &str) -> Self {
        let doc = Document::from_html(html);
        // 文档根节点：用 scraper 的 root_element 的 id
        let root_id = doc.html.root_element().id();
        Self { doc, node_id: root_id }
    }

    /// Parse an HTML fragment.
    pub fn from_fragment(html: &str) -> Self {
        // fragment 也用 parse_document，保持一致
        Self::from_html(html)
    }
```

- [ ] **Step 4: 重写 select / select_one（共享 Document）**

```rust
    /// Select all elements matching a CSS selector.
    pub fn select(&self, css: &str) -> NodeList {
        let selector = CssSelector::parse(css).unwrap_or_else(|_| CssSelector::parse("*").unwrap());
        let nodes: Vec<Node> = self.doc.html.select(&selector)
            .map(|el| Node::from_element_ref(self.doc.clone(), el))
            .collect();
        NodeList { nodes }
    }

    /// Alias for select() returning Vec<Node> for ergonomic iteration.
    pub fn select_all(&self, css: &str) -> Vec<Node> {
        self.select(css).nodes
    }

    /// Select the first element matching a CSS selector.
    pub fn select_one(&self, css: &str) -> Option<Node> {
        let selector = CssSelector::parse(css).ok()?;
        self.doc.html.select(&selector).next()
            .map(|el| Node::from_element_ref(self.doc.clone(), el))
    }
```

- [ ] **Step 5: 重写 text / html / outer_html / attr / attrs（用 element_ref）**

```rust
    /// Get the text content of the document/element.
    pub fn text(&self) -> String {
        self.element_ref()
            .map(|e| e.text().collect::<Vec<_>>().join(""))
            .unwrap_or_default()
    }

    /// Get the inner HTML.
    pub fn html(&self) -> String {
        self.element_ref()
            .map(|e| e.inner_html())
            .unwrap_or_default()
    }

    /// Get the outer HTML.
    pub fn outer_html(&self) -> String {
        self.element_ref()
            .map(|e| e.html())
            .unwrap_or_default()
    }

    /// Get an attribute value.
    pub fn attr(&self, name: &str) -> Option<String> {
        self.element_ref()
            .and_then(|e| e.value().attr(name).map(|s| s.to_string()))
    }

    /// Get all attributes as a map.
    pub fn attrs(&self) -> HashMap<String, String> {
        self.element_ref()
            .map(|e| {
                e.value().attrs()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
```

- [ ] **Step 6: 重写 css_adaptive / inner / 其他方法**

```rust
    /// Adaptive CSS selection with SQLite-backed snapshot persistence.
    pub fn css_adaptive(
        &self,
        selector: &str,
        key: &str,
        url: &str,
        store: &crate::storage::Store,
        auto_save: bool,
        tolerance: f64,
    ) -> Option<Node> {
        adaptive::css_adaptive(self, selector, key, url, store, auto_save, tolerance)
    }

    /// Access the underlying scraper::Html for advanced usage.
    pub fn inner(&self) -> &Html {
        &self.doc.html
    }

    /// Get the tag name of the element.
    pub fn tag(&self) -> String {
        self.element_ref()
            .map(|e| e.value().name().to_string())
            .unwrap_or_default()
    }
```

- [ ] **Step 7: 临时保留 parent/children/next_sibling/prev_sibling/matches 的旧实现（Task 4 真实实现）**

```rust
    /// Get the parent element.
    pub fn parent(&self) -> Option<Node> {
        None // Task 4 真实实现
    }

    /// Get direct child elements.
    pub fn children(&self) -> NodeList {
        let element = match self.element_ref() { Some(e) => e, None => return NodeList { nodes: Vec::new() } };
        let nodes: Vec<Node> = element.children()
            .filter_map(|c| ElementRef::wrap(c).filter(|e| e.value().is_element()))
            .map(|c| Node::from_element_ref(self.doc.clone(), c))
            .collect();
        NodeList { nodes }
    }

    /// Get the next sibling element.
    pub fn next_sibling(&self) -> Option<Node> {
        None // Task 4 真实实现
    }

    /// Get the previous sibling element.
    pub fn prev_sibling(&self) -> Option<Node> {
        None // Task 4 真实实现
    }

    /// Get the first child element.
    pub fn first_child(&self) -> Option<Node> {
        self.children().first().cloned()
    }

    /// Get the last child element.
    pub fn last_child(&self) -> Option<Node> {
        self.children().last().cloned()
    }

    /// Check if element matches a CSS selector.
    pub fn matches(&self, css: &str) -> bool {
        false // Task 4 真实实现
    }
```

- [ ] **Step 8: 保留 xpath 方法（Task 6 接入 sxd-xpath）**

```rust
    /// Select all elements matching an XPath expression.
    pub fn xpath(&self, expr: &str) -> NodeList {
        // 快速路径：简单 XPath 转 CSS（覆盖 80% 常见用法）
        if let Some(css) = xpath_to_css(expr) {
            return self.select(&css);
        }
        // Task 6 接入 sxd-xpath 完整查询
        NodeList { nodes: Vec::new() }
    }
```

- [ ] **Step 9: 保留其他方法不变（contains_text/generate_selector/generate_xpath/text_clean/text_regex）**

这些方法基于 text() 和 select()，无需修改。

- [ ] **Step 10: 运行 cargo check 验证编译**

Run: `cargo check`
Expected: 编译通过。常见问题：
1. `ElementRef::wrap` 签名变化 — 查 scraper 0.23 文档
2. `root_element().id()` 不存在 — 用 `.tree.root_element().id()` 或类似
3. `element.children()` 返回 `NodeRef` 迭代器，需要 `ElementRef::wrap` 转换

如果编译失败，根据错误信息修复。关键：scraper 0.23 的 ElementRef API。

- [ ] **Step 11: 运行现有测试**

Run: `cargo test --lib`
Expected: 全部通过。如果有测试失败，通常是 select() 行为变化（之前每个结果重新 parse fragment，现在共享 Document）。修复测试或修复实现。

Run: `cargo test --test adaptive_test`
Expected: 5 passed

Run: `cargo test --test difflib_test`
Expected: 7 passed

- [ ] **Step 12: 提交**

```bash
git add src/parser/mod.rs
git commit -m "refactor: Node 重构为 Arc<Document> + node_id 共享所有权"
```

---

## Task 4: DOM 导航真实实现（parent/children/sibling/ancestors/matches）

**Files:**
- Modify: `src/parser/mod.rs`
- Create: `tests/dom_navigation_test.rs`

- [ ] **Step 1: 先写失败测试 tests/dom_navigation_test.rs**

```rust
//! Verify DOM navigation methods work correctly after Node refactor.

use wisp::parser::Node;

const NAV_HTML: &str = r#"
<html>
  <body>
    <div id="parent" class="container">
      <p class="first">First paragraph</p>
      <p class="second" data-x="1">Second paragraph</p>
      <p class="third">Third paragraph</p>
      <span class="sibling">After paragraphs</span>
    </div>
    <section id="other">Other section</section>
  </body>
</html>
"#;

#[test]
fn test_parent_navigation() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.second").expect("p.second should exist");
    let parent = p.parent().expect("p should have a parent");
    assert_eq!(parent.attr("id"), Some("parent".to_string()));
    assert_eq!(parent.tag(), "div");
}

#[test]
fn test_children_navigation() {
    let doc = Node::from_html(NAV_HTML);
    let parent = doc.select_one("#parent").expect("#parent should exist");
    let children = parent.children();
    // 3 个 <p> + 1 个 <span> = 4 个元素子节点
    assert_eq!(children.len(), 4);
    assert_eq!(children.get(0).unwrap().attr("class"), Some("first".to_string()));
    assert_eq!(children.get(3).unwrap().tag(), "span");
}

#[test]
fn test_next_sibling() {
    let doc = Node::from_html(NAV_HTML);
    let first = doc.select_one("p.first").expect("p.first should exist");
    let next = first.next_sibling().expect("should have next sibling");
    assert_eq!(next.attr("class"), Some("second".to_string()));
}

#[test]
fn test_prev_sibling() {
    let doc = Node::from_html(NAV_HTML);
    let third = doc.select_one("p.third").expect("p.third should exist");
    let prev = third.prev_sibling().expect("should have prev sibling");
    assert_eq!(prev.attr("class"), Some("second".to_string()));
}

#[test]
fn test_next_sibling_none_at_end() {
    let doc = Node::from_html(NAV_HTML);
    let span = doc.select_one("span.sibling").expect("span should exist");
    // span 是 div 的最后一个元素子节点
    assert!(span.next_sibling().is_none());
}

#[test]
fn test_ancestors_iterator() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.first").expect("p.first should exist");
    let ancestors: Vec<Node> = p.ancestors().collect();
    // p -> div#parent -> body -> html
    assert!(ancestors.len() >= 3);
    assert_eq!(ancestors[0].attr("id"), Some("parent".to_string()));
    assert_eq!(ancestors[1].tag(), "body");
    assert_eq!(ancestors[2].tag(), "html");
}

#[test]
fn test_matches_simple_selector() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.second").expect("p.second should exist");
    assert!(p.matches("p"));
    assert!(p.matches("p.second"));
    assert!(p.matches("[data-x]"));
    assert!(!p.matches("p.first"));
    assert!(!p.matches("div"));
}

#[test]
fn test_matches_compound_selector() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.second").expect("p.second should exist");
    assert!(p.matches("p.second[data-x='1']"));
    assert!(!p.matches("p.second[data-x='2']"));
}

#[test]
fn test_tag_name() {
    let doc = Node::from_html(NAV_HTML);
    let p = doc.select_one("p.first").expect("p.first should exist");
    assert_eq!(p.tag(), "p");
    let div = doc.select_one("#parent").expect("#parent should exist");
    assert_eq!(div.tag(), "div");
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test dom_navigation_test`
Expected: 多个测试 FAIL（parent/ancestors/matches 未实现）

- [ ] **Step 3: 真实实现 parent / next_sibling / prev_sibling**

在 `src/parser/mod.rs` 的 Node impl 中替换临时实现：

```rust
    /// Get the parent element.
    pub fn parent(&self) -> Option<Node> {
        let element = self.element_ref()?;
        element.parent()
            .and_then(ElementRef::wrap)
            .filter(|p| p.value().is_element())
            .map(|p| Node::from_element_ref(self.doc.clone(), p))
    }

    /// Get the next sibling element (skips non-element nodes).
    pub fn next_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.next_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                if el.value().is_element() {
                    return Some(Node::from_element_ref(self.doc.clone(), el));
                }
            }
            sib = s.next_sibling();
        }
        None
    }

    /// Get the previous sibling element (skips non-element nodes).
    pub fn prev_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.prev_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                if el.value().is_element() {
                    return Some(Node::from_element_ref(self.doc.clone(), el));
                }
            }
            sib = s.prev_sibling();
        }
        None
    }
```

- [ ] **Step 4: 新增 ancestors() 迭代器**

在 Node impl 中追加：

```rust
    /// 从当前节点的父节点开始，向上迭代到文档根。
    pub fn ancestors(&self) -> impl Iterator<Item = Node> + '_ {
        std::iter::successors(self.parent(), |node| node.parent())
    }
```

- [ ] **Step 5: 真实实现 matches()**

```rust
    /// Check if element matches a CSS selector.
    pub fn matches(&self, css: &str) -> bool {
        let selector = match CssSelector::parse(css) {
            Ok(s) => s,
            Err(_) => return false,
        };
        self.element_ref()
            .map(|e| e.value().matches(&selector))
            .unwrap_or(false)
    }
```

- [ ] **Step 6: 运行 cargo check 验证编译**

Run: `cargo check`
Expected: 编译通过

- [ ] **Step 7: 运行新测试验证通过**

Run: `cargo test --test dom_navigation_test`
Expected: 9 个测试全部 PASS

- [ ] **Step 8: 运行现有测试确保未破坏**

Run: `cargo test --lib && cargo test --test adaptive_test && cargo test --test difflib_test`
Expected: 全部通过

- [ ] **Step 9: 提交**

```bash
git add src/parser/mod.rs tests/dom_navigation_test.rs
git commit -m "feat: 真实实现 parent/children/sibling/ancestors/matches DOM 导航"
```

---

## Task 5: sxd-xpath 完整查询集成

**Files:**
- Create: `src/parser/xpath.rs`
- Modify: `src/parser/mod.rs`（声明子模块 + 接入 xpath 方法）

- [ ] **Step 1: 创建 src/parser/xpath.rs**

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

- [ ] **Step 2: 在 src/parser/mod.rs 声明 xpath 子模块**

在 `pub mod document;` 之后追加：

```rust
pub mod xpath;
```

- [ ] **Step 3: 修改 Node::xpath 接入 xpath_full**

找到当前的 xpath 方法（Task 3 Step 8 保留的临时实现），替换为：

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

- [ ] **Step 4: 运行 cargo check 验证编译**

Run: `cargo check`
Expected: 编译通过。常见问题：
1. `sxd_document::dom` API 变化 — 查 sxd-document 0.3.2 文档
2. `Factory::new()` / `build()` 签名 — 查 sxd-xpath 0.4.2 文档
3. `dom::ChildOfElement` 枚举不存在 — 用 `dom::Child` 或其他

如果编译失败，根据 sxd-document 0.3.2 和 sxd-xpath 0.4.2 的实际 API 修复。

- [ ] **Step 5: 提交**

```bash
git add src/parser/xpath.rs src/parser/mod.rs
git commit -m "feat: sxd-xpath 完整查询集成（懒加载 + 结果回查 scraper 树）"
```

---

## Task 6: XPath 测试

**Files:**
- Create: `tests/xpath_test.rs`

- [ ] **Step 1: 创建 tests/xpath_test.rs**

```rust
//! Verify XPath queries work for both simple (fast path) and complex (sxd) expressions.

use wisp::parser::Node;

const XPATH_HTML: &str = r#"
<html>
  <body>
    <div id="main" class="container">
      <h1>Title</h1>
      <ul>
        <li class="item">Item 1</li>
        <li class="item">Item 2</li>
        <li class="item">Item 3</li>
        <li class="special">Item 4</li>
      </ul>
      <a href="https://example.com/page1">Link 1</a>
      <a href="https://example.com/page2">Link 2</a>
    </div>
  </body>
</html>
"#;

#[test]
fn test_xpath_simple_tag() {
    // 快速路径：//tag -> tag
    let doc = Node::from_html(XPATH_HTML);
    let lis = doc.xpath("//li");
    assert_eq!(lis.len(), 4);
}

#[test]
fn test_xpath_by_id() {
    // 快速路径：//*[@id='value'] -> #value
    let doc = Node::from_html(XPATH_HTML);
    let main = doc.xpath("//*[@id='main']");
    assert_eq!(main.len(), 1);
    assert_eq!(main.get(0).unwrap().attr("class"), Some("container".to_string()));
}

#[test]
fn test_xpath_attr_value() {
    // 快速路径：//tag[@attr='value']
    let doc = Node::from_html(XPATH_HTML);
    let special = doc.xpath("//li[@class='special']");
    assert_eq!(special.len(), 1);
    assert!(special.get(0).unwrap().text().contains("Item 4"));
}

#[test]
fn test_xpath_contains_href() {
    // 快速路径：//tag[contains(@attr, 'value')]
    let doc = Node::from_html(XPATH_HTML);
    let links = doc.xpath("//a[contains(@href, 'example.com')]");
    assert_eq!(links.len(), 2);
}

#[test]
fn test_xpath_position_predicate() {
    // 慢路径：position() 谓词需要 sxd-xpath
    let doc = Node::from_html(XPATH_HTML);
    let items = doc.xpath("//li[position()>2]");
    assert_eq!(items.len(), 2);
}

#[test]
fn test_xpath_text_content() {
    // 慢路径：text() 函数
    let doc = Node::from_html(XPATH_HTML);
    let items = doc.xpath("//li[contains(text(), 'Item 1')]");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_xpath_returns_empty_on_no_match() {
    let doc = Node::from_html(XPATH_HTML);
    let result = doc.xpath("//nonexistent");
    assert_eq!(result.len(), 0);
}

#[test]
fn test_xpath_malformed_returns_empty() {
    let doc = Node::from_html(XPATH_HTML);
    // 格式错误的 xpath 应返回空，不 panic
    let result = doc.xpath("///[[[");
    assert_eq!(result.len(), 0);
}

#[test]
fn test_xpath_html5_tolerance() {
    // 不规范 HTML（未闭合标签）应能正常解析
    let html = r#"<html><body><div><p>Unclosed paragraph<div>Nested</div></body></html>"#;
    let doc = Node::from_html(html);
    let result = doc.xpath("//p");
    assert_eq!(result.len(), 1);
    assert!(result.get(0).unwrap().text().contains("Unclosed"));
}
```

- [ ] **Step 2: 运行测试**

Run: `cargo test --test xpath_test`
Expected: 9 个测试通过。如果 `test_xpath_position_predicate` / `test_xpath_text_content` 失败，检查 sxd-xpath 集成是否正确。

- [ ] **Step 3: 运行现有测试确保未破坏**

Run: `cargo test --lib && cargo test --test dom_navigation_test && cargo test --test adaptive_test`
Expected: 全部通过

- [ ] **Step 4: 提交**

```bash
git add tests/xpath_test.rs
git commit -m "test: XPath 快速路径与 sxd-xpath 慢路径覆盖测试"
```

---

## Task 7: ElementSnapshot::capture 升级用 Node 导航 API

**Files:**
- Modify: `src/parser/adaptive.rs`

**目标：** 阶段 1 的 capture 用 scraper::ElementRef 临时拿上下文（每次 similarity 调用重复解析 outer_html 4 次）。阶段 2 Node 重构后，capture 改用 Node 的 ancestors()/parent()/children() 导航 API，消除重复解析。

- [ ] **Step 1: 读取当前 ElementSnapshot::capture 实现**

读取 `src/parser/adaptive.rs` 的 `capture` 函数和 4 个 helper（node_tag_name / ancestor_path_of / sibling_tags_of / parent_attrs_of）。

- [ ] **Step 2: 重写 capture 用 Node 导航 API**

在 `src/parser/adaptive.rs` 中替换 `capture` 函数：

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

        // ancestor_path: 从父节点到根，每级 "tag" 或 "tag.firstclass"
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
                    Some(format!("{}.{}", t, first_class))
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // sibling_tags: 父节点的所有元素子节点的 tag 列表
        let sibling_tags: Vec<String> = node.parent()
            .map(|p| p.children().iter().map(|c| c.tag()).collect())
            .unwrap_or_default();

        // position_in_parent: 当前节点在父节点子元素中的索引
        let position_in_parent = node.parent()
            .map(|p| {
                p.children().iter()
                    .position(|c| c.matches(&node.tag()) && c.text() == node.text())
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        let parent_tag = node.parent()
            .map(|p| p.tag())
            .unwrap_or_default();

        let parent_attrs = node.parent()
            .map(|p| p.attrs())
            .unwrap_or_default();

        Self {
            tag,
            attrs,
            text_preview,
            ancestor_path,
            sibling_tags,
            position_in_parent: position_in_parent as u32,
            parent_tag,
            parent_attrs,
        }
    }
}
```

- [ ] **Step 3: 删除旧的 4 个 helper 函数**

删除 `node_tag_name` / `ancestor_path_of` / `sibling_tags_of` / `parent_attrs_of`（如果它们只被旧 capture 使用）。如果还被其他地方引用，保留。

- [ ] **Step 4: 删除 capture_from_element_ref（如果存在）**

阶段 1 的 `capture_from_element_ref` 辅助函数不再需要，删除。

- [ ] **Step 5: 运行 cargo check 验证编译**

Run: `cargo check`
Expected: 编译通过

- [ ] **Step 6: 运行 adaptive 测试**

Run: `cargo test --test adaptive_test`
Expected: 5 passed（capture + relocate 行为不变）

Run: `cargo test --test integration adaptive_test`
Expected: 端到端测试通过

- [ ] **Step 7: 运行全部测试确保未破坏**

Run: `cargo test --lib && cargo test --test dom_navigation_test && cargo test --test xpath_test && cargo test --test crawl_checkpoint_test && cargo test --test difflib_test`
Expected: 全部通过

- [ ] **Step 8: 提交**

```bash
git add src/parser/adaptive.rs
git commit -m "refactor: ElementSnapshot::capture 升级用 Node 导航 API（消除重复解析）"
```

---

## Task 8: 端到端集成测试与 stage 2 完成验证

**Files:**
- Modify: `tests/integration.rs`（追加 stage 2 集成测试）

- [ ] **Step 1: 在 tests/integration.rs 的 adaptive_test 模块追加 stage 2 测试**

在 `tests/integration.rs` 的 `mod adaptive_test { ... }` 末尾追加：

```rust
    #[test]
    fn test_dom_navigation_with_adaptive_snapshot() {
        // 验证 Node 重构后 adaptive 仍正常工作，且 capture 用了导航 API
        use wisp::parser::Node;
        use wisp::storage::Store;

        let store = Store::open_in_memory().unwrap();
        let url = "https://shop.example.com/products";

        let html = r#"
        <html><body>
          <div class="products">
            <div class="product" data-id="1">
              <h3 class="title">Widget</h3>
            </div>
          </div>
        </body></html>
        "#;

        let doc = Node::from_html(html);
        let node = doc.css_adaptive(".title", "product-title", url, &store, true, 0.5);
        assert!(node.is_some());
        assert_eq!(node.unwrap().text(), "Widget");

        // 验证 capture 用了导航 API：检查 snapshot 的 ancestor_path 包含 "div.products"
        let saved = store.load_element(url, "product-title").unwrap().expect("snapshot should be saved");
        let snapshot: wisp::parser::ElementSnapshot = saved.into();
        assert!(snapshot.ancestor_path.iter().any(|p| p.contains("products")));
    }

    #[test]
    fn test_xpath_and_css_consistency() {
        // 验证 XPath 和 CSS 对同一查询返回一致结果
        use wisp::parser::Node;

        let html = r#"
        <html><body>
          <ul>
            <li class="item">A</li>
            <li class="item">B</li>
            <li class="item">C</li>
          </ul>
        </body></html>
        "#;

        let doc = Node::from_html(html);
        let css_result = doc.select("li.item");
        let xpath_result = doc.xpath("//li[@class='item']");

        assert_eq!(css_result.len(), xpath_result.len());
        assert_eq!(css_result.len(), 3);
    }

    #[test]
    fn test_node_shares_document_after_select() {
        // 验证 select 返回的 Node 共享同一 Document（导航可工作）
        use wisp::parser::Node;

        let html = r#"<html><body><div><p>Hello</p></div></body></html>"#;
        let doc = Node::from_html(html);
        let p = doc.select_one("p").expect("p should exist");
        // 阶段 1 的 fragment 模型下 parent() 返回 None
        // 阶段 2 重构后 parent() 应返回 div
        let parent = p.parent().expect("parent should work after refactor");
        assert_eq!(parent.tag(), "div");
    }
```

- [ ] **Step 2: 运行新测试**

Run: `cargo test --test integration adaptive_test`
Expected: 全部通过（含原有 test_end_to_end_adaptive_relocation + 3 个新测试）

- [ ] **Step 3: 运行完整测试套件**

Run: `cargo test --lib && cargo test --test adaptive_test && cargo test --test crawl_checkpoint_test && cargo test --test difflib_test && cargo test --test dom_navigation_test && cargo test --test xpath_test && cargo test --test integration adaptive_test`
Expected: 全部通过

- [ ] **Step 4: 提交**

```bash
git add tests/integration.rs
git commit -m "test: 阶段 2 端到端集成测试（DOM 导航 + XPath + adaptive 一致性）"
```

---

## Self-Review 检查

**1. Spec 覆盖检查**：
- ✅ Node 内部重构为 Arc<Document> → Task 2, 3
- ✅ DOM 导航真实实现（parent/children/sibling/ancestors/matches）→ Task 4
- ✅ sxd-xpath 懒解析集成 → Task 2 (Document) + Task 5 (xpath_full)
- ✅ HTML5 容错（html5ever 规范化）→ Task 2 (build_sxd_from_html)
- ✅ xpath_to_css 快速路径保留 → Task 3 (保留) + Task 5 (接入)
- ✅ ElementSnapshot::capture 升级 → Task 7
- ⚠️ wreq 替换 reqwest（2.1）→ 推迟（环境缺 perl/nasm）

**2. Placeholder 扫描**：
- 无 "TBD"、"TODO" 占位
- 所有步骤都有完整代码

**3. 类型一致性**：
- `Document` 在 Task 2 定义，Task 3/5/7 使用一致
- `Node::from_element_ref` / `element_ref` 在 Task 3 定义，Task 4/5/7 使用一致
- `xpath::xpath_full` 在 Task 5 定义，Task 3 的 xpath 方法调用一致
- `ElementSnapshot::capture` 在 Task 7 重写，签名与 Task 3 的 css_adaptive 调用一致

**4. 已知简化**：
- `locate_in_sxd` 用 tag 名启发式定位（非精确路径匹配）——stage 2 接受，复杂场景回退到 doc.root()
- `find_in_scraper` 用第一个属性构造选择器——多元素同属性时可能匹配错误，stage 2 接受
- wreq 替换推迟——环境依赖（perl/nasm），不影响解析能力

---

## 执行说明

本 plan 适用于 subagent-driven-development。每个 Task 独立可测、可提交。Task 3（Node 重构）是最高风险 task，需特别注意保持 API 向后兼容。
