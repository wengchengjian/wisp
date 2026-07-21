//! HTML parsing with CSS/XPath selectors.

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
    doc: Arc<Document>,
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
            let selector = CssSelector::parse(&inner_tag)
                .unwrap_or_else(|_| CssSelector::parse("*").unwrap());
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

    /// Select all elements matching a CSS selector.
    ///
    /// 注意：当前实现搜索整个文档（`self.doc.html.select`），不 scope 到当前 Node 的子树。
    /// 这是 Task 3 重构带来的语义变化（旧实现 scope 到子树）。Task 4 计划用
    /// `element_ref().select()` 实现 scoped 查询。
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

    /// Select all elements matching an XPath expression.
    ///
    /// Supports common XPath patterns:
    /// - `//tag` - all elements with tag name
    /// - `//tag[@attr='value']` - elements with attribute
    /// - `//tag[@attr]` - elements having attribute
    /// - `//*[@id='value']` - by ID
    /// - `//tag[contains(@class, 'value')]` - class contains
    pub fn xpath(&self, expr: &str) -> NodeList {
        // Convert common XPath patterns to CSS selectors
        if let Some(css) = xpath_to_css(expr) {
            return self.select(&css);
        }
        // Fallback: return empty for unsupported XPath
        NodeList { nodes: Vec::new() }
    }

    /// Get the parent element (returns None for now; Task 4 真实实现).
    pub fn parent(&self) -> Option<Node> {
        None // Task 4 真实实现
    }

    /// Get direct child elements.
    pub fn children(&self) -> NodeList {
        let element = match self.element_ref() { Some(e) => e, None => return NodeList { nodes: Vec::new() } };
        let nodes: Vec<Node> = element.child_elements()
            .map(|c| Node::from_element_ref(self.doc.clone(), c))
            .collect();
        NodeList { nodes }
    }

    /// Get the next sibling element (returns None for now; Task 4 真实实现).
    pub fn next_sibling(&self) -> Option<Node> {
        None // Task 4 真实实现
    }

    /// Get the previous sibling element (returns None for now; Task 4 真实实现).
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
    pub fn matches(&self, _css: &str) -> bool {
        false // Task 4 真实实现
    }

    /// Check if text content contains a substring.
    pub fn contains_text(&self, text: &str) -> bool {
        self.text().contains(text)
    }

    /// Generate a unique CSS selector for this element.
    pub fn generate_selector(&self) -> String {
        generate::generate_css(self)
    }

    /// Generate a unique XPath for this element.
    pub fn generate_xpath(&self) -> String {
        generate::generate_xpath(self)
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

/// Convert common XPath expressions to CSS selectors.
fn xpath_to_css(xpath: &str) -> Option<String> {
    let xpath = xpath.trim();

    // //tag[@attr='value']
    if let Some(rest) = xpath.strip_prefix("//") {
        // //*[@id='value'] -> #value
        if let Some(id) = extract_attr_value(rest, "id") {
            return Some(format!("#{}", id));
        }
        // //tag[@attr='value'] -> tag[attr='value']
        if let Some((tag, attr, value)) = parse_tag_attr_value(rest) {
            return Some(format!("{}[{}='{}']", tag, attr, value));
        }
        // //tag[@attr] -> tag[attr]
        if let Some((tag, attr)) = parse_tag_attr(rest) {
            return Some(format!("{}[{}]", tag, attr));
        }
        // //tag[contains(@class, 'value')] -> tag.value (approximate)
        if let Some((tag, class)) = parse_contains_class(rest) {
            return Some(format!("{}.{}", tag, class));
        }
        // //tag -> tag
        let tag: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '-').collect();
        if !tag.is_empty() && tag.len() == rest.len() {
            return Some(tag);
        }
        // //* -> *
        if rest == "*" {
            return Some("*".to_string());
        }
    }
    None
}

fn extract_attr_value(s: &str, attr: &str) -> Option<String> {
    // Pattern: *[@attr='value'] or tag[@attr='value']
    let pattern = format!("[@{}='", attr);
    if let Some(start) = s.find(&pattern) {
        let rest = &s[start + pattern.len()..];
        let value: String = rest.chars().take_while(|c| *c != '\'' && *c != '"').collect();
        if !value.is_empty() { return Some(value); }
    }
    None
}

fn parse_tag_attr_value(s: &str) -> Option<(String, String, String)> {
    // Pattern: tag[@attr='value']
    let bracket = s.find('[')?;
    let tag = &s[..bracket];
    if tag.is_empty() || tag == "*" { return None; }
    let rest = &s[bracket+1..];
    let at = rest.strip_prefix('@')?;
    let eq = at.find('=')?;
    let attr = &at[..eq];
    let val_part = &at[eq+1..];
    let value: String = val_part.chars().skip(1).take_while(|c| *c != '\'' && *c != '"' && *c != ']').collect();
    Some((tag.to_string(), attr.to_string(), value))
}

fn parse_tag_attr(s: &str) -> Option<(String, String)> {
    // Pattern: tag[@attr]
    let bracket = s.find('[')?;
    let tag = &s[..bracket];
    if tag.is_empty() || tag == "*" { return None; }
    let rest = &s[bracket+1..];
    let at = rest.strip_prefix('@')?;
    let attr: String = at.chars().take_while(|c| *c != ']').collect();
    if attr.is_empty() { return None; }
    Some((tag.to_string(), attr))
}

fn parse_contains_class(s: &str) -> Option<(String, String)> {
    // Pattern: tag[contains(@class, 'value')]
    let bracket = s.find('[')?;
    let tag = &s[..bracket];
    if tag.is_empty() { return None; }
    let rest = &s[bracket..];
    if rest.contains("contains(@class") {
        let quote_start = rest.find('\'').or_else(|| rest.find('"'))?;
        let val_start = &rest[quote_start+1..];
        let value: String = val_start.chars().take_while(|c| *c != '\'' && *c != '"').collect();
        return Some((tag.to_string(), value));
    }
    None
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
    fn test_generate_xpath() {
        let node = Node::from_fragment(r#"<div id="unique">Content</div>"#);
        assert_eq!(node.generate_xpath(), "//*[@id=\"unique\"]");
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
}
