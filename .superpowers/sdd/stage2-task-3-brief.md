# Task 3: Node 重构为 Arc<Document> + node_id

**Files:**
- Modify: `src/parser/mod.rs`（重写 Node struct + 所有方法）
- 可能 Modify: `src/parser/document.rs`（如果 OnceCell 导致 Sync 问题，改为 OnceLock）

**关键约束：** 所有公开 API 签名保持不变（from_html/from_fragment/select/select_one/text/attr/html/outer_html/css_adaptive 等）。现有测试必须全部通过。

## Step 1: 先跑现有测试确认基线

Run: `cargo test --lib`
Expected: 34 passed（阶段 1 后的基线）

## Step 2: 重写 src/parser/mod.rs 的 Node struct 定义

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

## Step 3: 重写 from_html / from_fragment

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

## Step 4: 重写 select / select_one（共享 Document）

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

## Step 5: 重写 text / html / outer_html / attr / attrs（用 element_ref）

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

## Step 6: 重写 css_adaptive / inner / 其他方法

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

## Step 7: 临时保留 parent/children/next_sibling/prev_sibling/matches 的旧实现（Task 4 真实实现）

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

## Step 8: 保留 xpath 方法（Task 6 接入 sxd-xpath）

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

## Step 9: 保留其他方法不变（contains_text/generate_selector/generate_xpath/text_clean/text_regex）

这些方法基于 text() 和 select()，无需修改。

## Step 10: 运行 cargo check 验证编译

Run: `cargo check`
Expected: 编译通过。常见问题：
1. `ElementRef::wrap` 签名变化 — 查 scraper 0.23 文档
2. `root_element().id()` 不存在 — 用 `.tree.root_element().id()` 或类似
3. `element.children()` 返回 `NodeRef` 迭代器，需要 `ElementRef::wrap` 转换

如果编译失败，根据错误信息修复。关键：scraper 0.23 的 ElementRef API。

## Step 11: 运行现有测试

Run: `cargo test --lib`
Expected: 全部通过。如果有测试失败，通常是 select() 行为变化（之前每个结果重新 parse fragment，现在共享 Document）。修复测试或修复实现。

Run: `cargo test --test adaptive_test`
Expected: 5 passed

Run: `cargo test --test difflib_test`
Expected: 7 passed

## Step 12: 提交

```bash
git add src/parser/mod.rs
git commit -m "refactor: Node 重构为 Arc<Document> + node_id 共享所有权"
```
