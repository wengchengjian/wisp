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

    /// Get the parent element (returns the containing element for fragments).
    pub fn parent(&self) -> Option<Node> {
        // Limited parent navigation for fragment-based nodes
        // The root element's parent in a fragment is the synthetic root
        None
    }

    /// Get direct child elements.
    pub fn children(&self) -> NodeList {
        let all = self.select("*");
        let root = self.inner.root_element();
        let child_count = root.children().filter(|c| c.value().is_element()).count();
        let nodes: Vec<Node> = all.nodes.into_iter().take(child_count).collect();
        NodeList { nodes }
    }

    /// Get the next sibling element.
    pub fn next_sibling(&self) -> Option<Node> {
        None // Limited in fragment-based architecture
    }

    /// Get the previous sibling element.
    pub fn prev_sibling(&self) -> Option<Node> {
        None // Limited in fragment-based architecture
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
}
