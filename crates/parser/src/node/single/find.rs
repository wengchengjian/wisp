//! 文本查找、相似元素定位与高级内容提取。

use super::*;

impl Node {
    /// 按文本内容查找元素。
    pub fn find_by_text(&self, text: &str, tag: Option<&str>, exact: bool) -> NodeList {
        let selector_str = match tag {
            Some(t) => t.to_string(),
            None => "*".to_string(),
        };
        let candidates = self.select(&selector_str);
        let matched: Vec<Node> = candidates
            .nodes
            .into_iter()
            .filter(|node| {
                let node_text = node.text();
                if exact {
                    node_text.trim() == text.trim()
                } else {
                    node_text.contains(text)
                }
            })
            .collect();
        NodeList { nodes: matched }
    }

    /// 查找与当前元素结构相似的同级元素。
    pub fn find_similar(&self) -> NodeList {
        let tag = self.tag();
        let attrs = self.attrs();
        let class_count = attrs
            .get("class")
            .map(|c| c.split_whitespace().count())
            .unwrap_or(0);
        let parent = match self.parent() {
            Some(p) => p,
            None => return NodeList { nodes: Vec::new() },
        };
        let similar: Vec<Node> = parent
            .children()
            .nodes
            .into_iter()
            .filter(|sibling| {
                // O(1) 比较 node_id 排除自身，避免生成完整 HTML 字符串
                if sibling.node_id == self.node_id {
                    return false;
                }
                if sibling.tag() != tag {
                    return false;
                }
                let sib_class_count = sibling
                    .attrs()
                    .get("class")
                    .map(|c| c.split_whitespace().count())
                    .unwrap_or(0);
                if (sib_class_count as i32 - class_count as i32).abs() > 1 {
                    return false;
                }
                let sib_attr_count = sibling.attrs().len();
                if (sib_attr_count as i32 - attrs.len() as i32).abs() > 2 {
                    return false;
                }
                true
            })
            .collect();
        NodeList { nodes: similar }
    }

    /// Generate a unique CSS selector for this element.
    pub fn generate_selector(&self) -> String {
        crate::generate::generate_css(self)
    }

    /// Get clean text (whitespace collapsed).
    pub fn text_clean(&self) -> String {
        wisp_core::text::Text(&self.text()).clean()
    }

    /// Extract regex matches from text.
    pub fn text_regex(&self, pattern: &str) -> Vec<String> {
        wisp_core::text::Text(&self.text()).extract_regex(pattern)
    }

    /// Access the underlying woven-html document for advanced usage.
    pub fn inner(&self) -> &woven_html::Document {
        &self.doc.html
    }
}
