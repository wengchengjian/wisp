//! HTML parsing with CSS selectors.

pub mod difflib;
pub mod document;
pub mod adaptive;
pub mod generate;

pub use adaptive::{
    ElementSnapshot, similarity, relocate_with_snapshot,
    css_adaptive, DEFAULT_TOLERANCE,
};

use scraper::{Html, Selector as CssSelector, ElementRef};
use std::collections::HashMap;
use std::sync::Arc;
use ego_tree::NodeId;
use document::Document;

/// A parsed HTML document or element.
///
/// 内部通过 `Arc<Document>` 共享文档所有权，`node_id` 标识在 scraper 树中的位置。
/// 所有 select() 返回的 Node 共享同一文档，使 parent/ancestors 等导航可工作。
#[derive(Clone)]
pub struct Node {
    pub(crate) doc: Arc<Document>,
    node_id: NodeId,
}

impl Node {
    /// 从 ElementRef 创建 Node（内部辅助方法）。
    fn from_element_ref(doc: Arc<Document>, el: ElementRef) -> Self {
        Self { doc, node_id: el.id() }
    }

    /// 获取当前节点对应的 ElementRef（在 scraper 树中查找）。
    fn element_ref(&self) -> Option<ElementRef<'_>> {
        ElementRef::wrap(self.doc.html.tree.get(self.node_id)?)
    }

    /// Parse HTML string into a Node (document root).
    pub fn from_html(html: &str) -> Self {
        let doc = Document::from_html(html);
        let root_id = doc.html.root_element().id();
        Self { doc, node_id: root_id }
    }

    /// Parse an HTML fragment.
    ///
    /// 普通元素片段用 `Html::parse_fragment`（保留片段语义，不创建 `<html><head><body>` 结构）。
    /// 表格元素片段（`<td>/<tr>/<th>/<thead>/<tbody>/<tfoot>/<caption>/<colgroup>/<col>`）
    /// 需要包裹 `<table>` 后用 `Html::parse_document` 解析，因为 HTML5 规范下这些表格元素
    /// 在 `<body>` context 中不合法，html5ever 会丢弃标签（只保留文本内容）。
    /// 包裹 `<table>` 后 html5ever 会规范化为 `<table><tbody><tr><td>...</td></tr></tbody></table>`，
    /// 保留表格元素标签，然后用选择器深入找到实际的片段元素。
    ///
    /// 注意：裸文本/注释片段在 `html > *` 不匹配元素时会回退到 root_element
    /// （此时 `tag()` 返回 `<html>`，可能不是用户期望的结果）。
    pub fn from_fragment(html: &str) -> Self {
        let trimmed_lower = html.trim_start().to_lowercase();
        // 提取片段开头的标签名（如 `<td>...` → "td"）
        let inner_tag: String = trimmed_lower
            .trim_start_matches('<')
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect();

        // 表格元素片段需要特殊处理
        let is_table_fragment = matches!(
            inner_tag.as_str(),
            "td" | "tr" | "th" | "thead" | "tbody" | "tfoot"
                | "caption" | "colgroup" | "col"
        );

        if is_table_fragment {
            // 表格元素包裹 <table> 后用 parse_document（html5ever 会规范化表格结构），
            // 然后用原始片段的标签名作为选择器，深入找到实际的片段元素。
            let wrapped = format!("<table>{}</table>", html);
            let doc = Document::from_html(&wrapped);
            // 标签名非法时回退到 root_element，不再静默回退到 *（避免匹配全部元素）
            let selector = match CssSelector::parse(&inner_tag) {
                Ok(s) => s,
                Err(_) => {
                    let root_id = doc.html.root_element().id();
                    return Self { doc, node_id: root_id };
                }
            };
            let root_id = doc.html.select(&selector)
                .next()
                .map(|el| el.id())
                .unwrap_or_else(|| doc.html.root_element().id());
            Self { doc, node_id: root_id }
        } else {
            // 普通元素用 parse_fragment（保留片段语义，不创建 <html><head><body> 结构）。
            // parse_fragment 创建 `<html>` root_element，片段内容直接在其下。
            let doc = Document::from_fragment(html);
            let selector = CssSelector::parse("html > *").unwrap();
            let root_id = doc.html.select(&selector)
                .next()
                .map(|el| el.id())
                .unwrap_or_else(|| doc.html.root_element().id());
            Self { doc, node_id: root_id }
        }
    }

    /// Select all elements matching a CSS selector, scoped to this node's subtree.
    ///
    /// 使用 `element_ref().select()` 实现 scoped 查询，仅搜索当前节点的子孙元素。
    /// 对文档根节点（`from_html` 创建），等价于搜索整个文档。
    pub fn select(&self, css: &str) -> NodeList {
        // 非法选择器返回空（与 select_one 返回 None 一致），不再静默回退到 *
        let Ok(selector) = CssSelector::parse(css) else {
            return NodeList { nodes: Vec::new() };
        };
        let nodes: Vec<Node> = match self.element_ref() {
            Some(el) => el.select(&selector)
                .map(|child| Node::from_element_ref(self.doc.clone(), child))
                .collect(),
            None => vec![],
        };
        NodeList { nodes }
    }

    /// Alias for select() returning Vec<Node> for ergonomic iteration.
    pub fn select_all(&self, css: &str) -> Vec<Node> {
        self.select(css).nodes
    }

    /// Select the first element matching a CSS selector, scoped to this node's subtree.
    pub fn select_one(&self, css: &str) -> Option<Node> {
        let selector = CssSelector::parse(css).ok()?;
        self.element_ref()?.select(&selector).next()
            .map(|el| Node::from_element_ref(self.doc.clone(), el))
    }

    /// Adaptive CSS selection with SQLite-backed snapshot persistence.
    ///
    /// See `adaptive::css_adaptive` for details.
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

    /// Get the tag name of the element.
    pub fn tag(&self) -> String {
        self.element_ref()
            .map(|e| e.value().name().to_string())
            .unwrap_or_default()
    }

    /// Get the parent element.
    ///
    /// 使用 `ElementRef::wrap` 过滤非元素节点（scraper 0.23 中 `Element` 没有 `is_element()`，
    /// 但 `ElementRef::wrap` 本身只对元素节点返回 `Some`）。
    pub fn parent(&self) -> Option<Node> {
        let element = self.element_ref()?;
        element.parent()
            .and_then(ElementRef::wrap)
            .map(|p| Node::from_element_ref(self.doc.clone(), p))
    }

    /// Get direct child elements.
    pub fn children(&self) -> NodeList {
        let element = match self.element_ref() { Some(e) => e, None => return NodeList { nodes: Vec::new() } };
        let nodes: Vec<Node> = element.child_elements()
            .map(|c| Node::from_element_ref(self.doc.clone(), c))
            .collect();
        NodeList { nodes }
    }

    /// Get the next sibling element (skips non-element nodes like text/comment).
    pub fn next_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.next_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                return Some(Node::from_element_ref(self.doc.clone(), el));
            }
            sib = s.next_sibling();
        }
        None
    }

    /// Get the previous sibling element (skips non-element nodes like text/comment).
    pub fn prev_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.prev_sibling();
        while let Some(s) = sib {
            if let Some(el) = ElementRef::wrap(s) {
                return Some(Node::from_element_ref(self.doc.clone(), el));
            }
            sib = s.prev_sibling();
        }
        None
    }

    /// Get the first child element.
    pub fn first_child(&self) -> Option<Node> {
        self.children().first().cloned()
    }

    /// Get the last child element.
    pub fn last_child(&self) -> Option<Node> {
        self.children().last().cloned()
    }

    /// Iterate ancestor elements from parent up to document root.
    ///
    /// 使用 `std::iter::successors` 链式调用 `parent()`，惰性迭代。
    pub fn ancestors(&self) -> impl Iterator<Item = Node> + '_ {
        std::iter::successors(self.parent(), |node| node.parent())
    }

    /// Check if element matches a CSS selector.
    ///
    /// 无效选择器返回 `false`（不 panic）。scraper 0.23 中 `Selector` 提供
    /// `matches(&ElementRef) -> bool` 方法（注意：方法在 `Selector` 上，不在 `Element` 上）。
    pub fn matches(&self, css: &str) -> bool {
        let selector = match CssSelector::parse(css) {
            Ok(s) => s,
            Err(_) => return false,
        };
        self.element_ref()
            .map(|e| selector.matches(&e))
            .unwrap_or(false)
    }

    /// Check if text content contains a substring.
    pub fn contains_text(&self, text: &str) -> bool {
        self.text().contains(text)
    }

    /// 按文本内容查找元素。
    pub fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList {
        let selector_str = match tag {
            Some(t) => t.to_string(),
            None => "*".to_string(),
        };
        let candidates = self.select(&selector_str);
        let matched: Vec<Node> = candidates.nodes.into_iter().filter(|node| {
            let node_text = node.text();
            if exact { node_text.trim() == text.trim() } else { node_text.contains(text) }
        }).collect();
        NodeList { nodes: matched }
    }

    /// 查找与当前元素结构相似的同级元素。
    pub fn find_similar(&self) -> NodeList {
        let tag = self.tag();
        let attrs = self.attrs();
        let class_count = attrs.get("class").map(|c| c.split_whitespace().count()).unwrap_or(0);
        let parent = match self.parent() {
            Some(p) => p,
            None => return NodeList { nodes: Vec::new() },
        };
        let similar: Vec<Node> = parent.children().nodes.into_iter().filter(|sibling| {
            if sibling.outer_html() == self.outer_html() { return false; }
            if sibling.tag() != tag { return false; }
            let sib_class_count = sibling.attrs().get("class").map(|c| c.split_whitespace().count()).unwrap_or(0);
            if (sib_class_count as i32 - class_count as i32).abs() > 1 { return false; }
            let sib_attr_count = sibling.attrs().len();
            if (sib_attr_count as i32 - attrs.len() as i32).abs() > 2 { return false; }
            true
        }).collect();
        NodeList { nodes: similar }
    }

    /// Generate a unique CSS selector for this element.
    pub fn generate_selector(&self) -> String {
        generate::generate_css(self)
    }

    /// Get clean text (whitespace collapsed).
    pub fn text_clean(&self) -> String {
        crate::text::Text(&self.text()).clean()
    }

    /// Extract regex matches from text.
    pub fn text_regex(&self, pattern: &str) -> Vec<String> {
        crate::text::Text(&self.text()).extract_regex(pattern)
    }

    /// Access the underlying scraper::Html for advanced usage.
    pub fn inner(&self) -> &Html {
        &self.doc.html
    }
}

/// A collection of DOM nodes.
#[derive(Clone)]
pub struct NodeList {
    pub(crate) nodes: Vec<Node>,
}

impl NodeList {
    pub fn new(nodes: Vec<Node>) -> Self { Self { nodes } }
    pub fn len(&self) -> usize { self.nodes.len() }
    pub fn is_empty(&self) -> bool { self.nodes.is_empty() }
    pub fn first(&self) -> Option<&Node> { self.nodes.first() }
    pub fn last(&self) -> Option<&Node> { self.nodes.last() }
    pub fn get(&self, index: usize) -> Option<&Node> { self.nodes.get(index) }

    /// Get text of all nodes.
    pub fn text(&self) -> Vec<String> { self.nodes.iter().map(|n| n.text()).collect() }

    /// Get HTML of all nodes.
    pub fn html(&self) -> Vec<String> { self.nodes.iter().map(|n| n.html()).collect() }

    /// Get an attribute from all nodes.
    pub fn attr(&self, name: &str) -> Vec<Option<String>> {
        self.nodes.iter().map(|n| n.attr(name)).collect()
    }

    /// Select within all nodes (union of results).
    pub fn select(&self, css: &str) -> NodeList {
        let mut results = Vec::new();
        for node in &self.nodes {
            results.extend(node.select(css).nodes);
        }
        NodeList { nodes: results }
    }

    /// Filter nodes by predicate.
    pub fn filter(&self, predicate: impl Fn(&Node) -> bool) -> NodeList {
        NodeList { nodes: self.nodes.iter().filter(|n| predicate(n)).cloned().collect() }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Node> { self.nodes.iter() }
}

impl IntoIterator for NodeList {
    type Item = Node;
    type IntoIter = std::vec::IntoIter<Node>;
    fn into_iter(self) -> Self::IntoIter { self.nodes.into_iter() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_html_and_text() {
        let doc = Node::from_html("<html><body><h1>Hello World</h1></body></html>");
        assert!(doc.text().contains("Hello World"));
    }

    #[test]
    fn test_select() {
        let doc = Node::from_html(r#"<html><body>
            <div class="item">First</div>
            <div class="item">Second</div>
        </body></html>"#);
        let items = doc.select("div.item");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_select_one() {
        let doc = Node::from_html(r#"<html><body>
            <p id="main">Content here</p>
        </body></html>"#);
        let p = doc.select_one("#main");
        assert!(p.is_some());
        assert!(p.unwrap().text().contains("Content here"));
    }

    #[test]
    fn test_attr() {
        let node = Node::from_fragment(r#"<a href="https://example.com" class="link">Click</a>"#);
        assert_eq!(node.attr("href"), Some("https://example.com".to_string()));
        assert_eq!(node.attr("class"), Some("link".to_string()));
        assert_eq!(node.attr("nonexistent"), None);
    }

    #[test]
    fn test_attrs() {
        let node = Node::from_fragment(r#"<div id="test" data-x="1">Content</div>"#);
        let attrs = node.attrs();
        assert_eq!(attrs.get("id"), Some(&"test".to_string()));
        assert_eq!(attrs.get("data-x"), Some(&"1".to_string()));
    }

    #[test]
    fn test_html() {
        let node = Node::from_fragment(r#"<div><span>inner</span></div>"#);
        let html = node.html();
        assert!(html.contains("<span>inner</span>"));
    }

    #[test]
    fn test_outer_html() {
        let node = Node::from_fragment(r#"<div class="x">text</div>"#);
        let outer = node.outer_html();
        assert!(outer.contains("class=\"x\""));
    }

    #[test]
    fn test_contains_text() {
        let doc = Node::from_html("<html><body><p>Hello World</p></body></html>");
        assert!(doc.contains_text("Hello"));
        assert!(!doc.contains_text("Goodbye"));
    }

    #[test]
    fn test_node_list_text() {
        let doc = Node::from_html(r#"<html><body>
            <li>A</li><li>B</li><li>C</li>
        </body></html>"#);
        let texts = doc.select("li").text();
        assert_eq!(texts, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_node_list_filter() {
        let doc = Node::from_html(r#"<html><body>
            <div class="keep">Keep1</div>
            <div class="drop">Drop</div>
            <div class="keep">Keep2</div>
        </body></html>"#);
        let filtered = doc.select("div").filter(|n| {
            n.attr("class") == Some("keep".to_string())
        });
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_generate_selector() {
        let node = Node::from_fragment(r#"<div id="unique">Content</div>"#);
        assert_eq!(node.generate_selector(), "#unique");
    }

    #[test]
    fn test_from_fragment_table_element() {
        // 表格元素片段不应被强制包裹 <table>（Important 2 回归测试）
        // 旧 Task 3 重构后用 parse_document 会让 tag() 返回 "table"；
        // 修复后用 parse_fragment，tag() 应返回 "td"。
        let node = Node::from_fragment("<td>cell</td>");
        assert_eq!(node.tag(), "td");
        assert!(node.text().contains("cell"));
        assert!(node.outer_html().contains("<td>cell</td>"));
    }

    #[test]
    fn select_invalid_selector_returns_empty_not_all() {
        let doc = Node::from_html(r#"<html><body><p>a</p><p>b</p></body></html>"#);
        // 非法选择器（未闭合括号）
        let nodes = doc.select("p[onclick=alert(");
        assert!(nodes.iter().count() == 0,
            "非法选择器应返回空，实际返回 {} 个（静默回退到 * 会返回 2 个 <p>）",
            nodes.iter().count());
    }

    #[test]
    fn select_valid_selector_still_works() {
        let doc = Node::from_html(r#"<html><body><p>a</p><p>b</p></body></html>"#);
        let nodes = doc.select("p");
        assert_eq!(nodes.iter().count(), 2);
    }
}
