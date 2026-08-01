//! CSS 选择与内容/属性查询。

use super::*;
use std::collections::HashMap;

impl Node {
    /// Select all elements matching a CSS selector, scoped to this node's subtree.
    ///
    /// 使用 `html_node().query_selector()` 实现 scoped 查询，仅搜索当前节点的子孙元素。
    /// 对文档根节点（`from_html` 创建），等价于搜索整个文档。
    pub fn select(&self, css: &str) -> NodeList {
        // 非法选择器返回空（与 select_one 返回 None 一致），不再静默回退到 *
        let nodes: Vec<Node> = match self.html_node() {
            Some(node) => node
                .query_selector(css)
                .unwrap_or_default()
                .into_iter()
                .map(|child| Node::from_html_node(self.doc.clone(), child))
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
        self.html_node()?
            .query_selector(css)
            .ok()?
            .into_iter()
            .next()
            .map(|el| Node::from_html_node(self.doc.clone(), el))
    }

    /// Get the text content of the document/element.
    pub fn text(&self) -> String {
        self.html_node()
            .map(|e| e.text_content())
            .unwrap_or_default()
    }

    /// Get the inner HTML.
    pub fn html(&self) -> String {
        self.html_node().map(|e| e.inner_html()).unwrap_or_default()
    }

    /// Get the outer HTML.
    pub fn outer_html(&self) -> String {
        self.html_node().map(|e| e.outer_html()).unwrap_or_default()
    }

    /// Get an attribute value.
    pub fn attr(&self, name: &str) -> Option<String> {
        self.html_node()
            .and_then(|e| e.get_attribute(name).map(str::to_string))
    }

    /// Get all attributes as a map.
    pub fn attrs(&self) -> HashMap<String, String> {
        self.html_node()
            .map(|e| {
                e.attributes()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the tag name of the element.
    pub fn tag(&self) -> String {
        self.html_node()
            .and_then(|e| e.local_name().map(str::to_string))
            .unwrap_or_default()
    }
}
