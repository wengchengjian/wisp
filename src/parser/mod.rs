//! HTML parsing with CSS/XPath selectors.

pub mod adaptive;
pub mod generate;

use scraper::{Html, Selector as CssSelector};
use std::collections::HashMap;

/// A parsed HTML document or element.
#[derive(Clone)]
pub struct Node {
    inner: Html,
    /// For fragments, store the first element's HTML to extract attrs correctly
    element_html: Option<String>,
}

impl Node {
    /// Parse HTML string into a Node (document root).
    pub fn from_html(html: &str) -> Self {
        Self { inner: Html::parse_document(html), element_html: None }
    }

    /// Parse an HTML fragment.
    pub fn from_fragment(html: &str) -> Self {
        let inner = Html::parse_fragment(html);
        // Store the first element child's HTML for attribute access
        let element_html = inner.root_element()
            .select(&CssSelector::parse("*").unwrap())
            .next()
            .map(|el| el.html());
        Self { inner, element_html }
    }

    /// Select all elements matching a CSS selector.
    pub fn select(&self, css: &str) -> NodeList {
        let selector = CssSelector::parse(css).unwrap_or_else(|_| CssSelector::parse("*").unwrap());
        let nodes: Vec<Node> = self.inner.select(&selector)
            .map(|el| {
                let html = el.html();
                Node {
                    inner: Html::parse_fragment(&html),
                    element_html: Some(html),
                }
            })
            .collect();
        NodeList { nodes }
    }

    /// Select the first element matching a CSS selector.
    pub fn select_one(&self, css: &str) -> Option<Node> {
        let selector = CssSelector::parse(css).ok()?;
        self.inner.select(&selector).next().map(|el| {
            let html = el.html();
            Node {
                inner: Html::parse_fragment(&html),
                element_html: Some(html),
            }
        })
    }

    /// Get the text content of the document/element.
    pub fn text(&self) -> String {
        self.inner.root_element().text().collect::<Vec<_>>().join("")
    }

    /// Get the inner HTML.
    pub fn html(&self) -> String {
        self.inner.root_element().inner_html()
    }

    /// Get the outer HTML.
    pub fn outer_html(&self) -> String {
        self.inner.root_element().html()
    }

    /// Get an attribute value (only works on fragment single-element nodes).
    pub fn attr(&self, name: &str) -> Option<String> {
        // For fragments, get first element child's attribute
        if let Some(ref html) = self.element_html {
            let frag = Html::parse_fragment(html);
            if let Some(el) = frag.root_element().select(&CssSelector::parse("*").unwrap()).next() {
                return el.value().attr(name).map(|s| s.to_string());
            }
        }
        self.inner.root_element().value().attr(name).map(|s| s.to_string())
    }

    /// Get all attributes as a map.
    pub fn attrs(&self) -> HashMap<String, String> {
        if let Some(ref html) = self.element_html {
            let frag = Html::parse_fragment(html);
            if let Some(el) = frag.root_element().select(&CssSelector::parse("*").unwrap()).next() {
                return el.value().attrs()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
            }
        }
        self.inner.root_element().value().attrs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Check if element matches a CSS selector.
    pub fn matches(&self, css: &str) -> bool {
        // Simplified: check if re-selecting from this node finds itself
        if CssSelector::parse(css).is_ok() {
            // For now, just check if the selector is valid
            // Full implementation would require parent context
            false
        } else {
            false
        }
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
        &self.inner
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
    fn test_generate_xpath() {
        let node = Node::from_fragment(r#"<div id="unique">Content</div>"#);
        assert_eq!(node.generate_xpath(), "//*[@id=\"unique\"]");
    }
}
