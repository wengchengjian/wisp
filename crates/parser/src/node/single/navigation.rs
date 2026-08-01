//! DOM 树导航与匹配。

use super::*;

impl Node {
    /// Get the parent element.
    ///
    /// woven-html 的 `parent()` 可能返回文本/文档节点，这里过滤为元素节点。
    pub fn parent(&self) -> Option<Node> {
        let node = self.html_node()?;
        node.parent()
            .filter(|p| p.is_element())
            .map(|p| Node::from_html_node(self.doc.clone(), p))
    }

    /// Get direct child elements.
    pub fn children(&self) -> NodeList {
        let node = match self.html_node() {
            Some(e) => e,
            None => return NodeList { nodes: Vec::new() },
        };
        let nodes: Vec<Node> = node
            .children()
            .into_iter()
            .filter(|c| c.is_element())
            .map(|c| Node::from_html_node(self.doc.clone(), c))
            .collect();
        NodeList { nodes }
    }

    /// Get the next sibling element (skips non-element nodes like text/comment).
    pub fn next_sibling(&self) -> Option<Node> {
        let node = self.html_node()?;
        let mut sib = node.next_sibling();
        while let Some(s) = sib {
            if s.is_element() {
                return Some(Node::from_html_node(self.doc.clone(), s));
            }
            sib = s.next_sibling();
        }
        None
    }

    /// Get the previous sibling element (skips non-element nodes like text/comment).
    pub fn prev_sibling(&self) -> Option<Node> {
        let node = self.html_node()?;
        let mut sib = node.previous_sibling();
        while let Some(s) = sib {
            if s.is_element() {
                return Some(Node::from_html_node(self.doc.clone(), s));
            }
            sib = s.previous_sibling();
        }
        None
    }

    /// Get the first child element.
    ///
    /// 直接遍历子元素取第一个，避免构造完整 Vec。
    pub fn first_child(&self) -> Option<Node> {
        let node = self.html_node()?;
        node.children()
            .into_iter()
            .find(|c| c.is_element())
            .map(|c| Node::from_html_node(self.doc.clone(), c))
    }

    /// Get the last child element.
    ///
    /// 直接遍历子元素取最后一个，避免构造完整 Vec。
    pub fn last_child(&self) -> Option<Node> {
        let node = self.html_node()?;
        node.children()
            .into_iter()
            .rev()
            .find(|c| c.is_element())
            .map(|c| Node::from_html_node(self.doc.clone(), c))
    }

    /// Iterate ancestor elements from parent up to document root.
    ///
    /// 使用 `std::iter::successors` 链式调用 `parent()`，惰性迭代。
    pub fn ancestors(&self) -> impl Iterator<Item = Node> + '_ {
        std::iter::successors(self.parent(), |node| node.parent())
    }

    /// Check if element matches a CSS selector.
    ///
    /// 无效选择器返回 `false`（不 panic）。woven-html 的 `Node::matches` 返回
    /// `Result<bool, SelectorError>`，错误统一视为不匹配。
    pub fn matches(&self, css: &str) -> bool {
        let Some(node) = self.html_node() else {
            return false;
        };
        node.matches(css).unwrap_or(false)
    }

    /// Check if text content contains a substring.
    pub fn contains_text(&self, text: &str) -> bool {
        self.text().contains(text)
    }
}
