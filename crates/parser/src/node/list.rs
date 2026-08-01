//! NodeList — batch DOM node collection.

use super::Node;

/// A collection of DOM nodes.
#[derive(Clone)]
pub struct NodeList {
    pub(crate) nodes: Vec<Node>,
}

impl NodeList {
    /// 创建节点列表。
    pub fn new(nodes: Vec<Node>) -> Self {
        Self { nodes }
    }
    /// 节点数量。
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    /// 获取第一个节点。
    pub fn first(&self) -> Option<&Node> {
        self.nodes.first()
    }
    /// 获取最后一个节点。
    pub fn last(&self) -> Option<&Node> {
        self.nodes.last()
    }
    /// 按索引获取节点。
    pub fn get(&self, index: usize) -> Option<&Node> {
        self.nodes.get(index)
    }

    /// Get text of all nodes.
    pub fn text(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.text()).collect()
    }

    /// Get HTML of all nodes.
    pub fn html(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.html()).collect()
    }

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
        NodeList {
            nodes: self
                .nodes
                .iter()
                .filter(|n| predicate(n))
                .cloned()
                .collect(),
        }
    }

    /// 返回节点迭代器。
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }
}

impl IntoIterator for NodeList {
    type Item = Node;
    type IntoIter = std::vec::IntoIter<Node>;
    fn into_iter(self) -> Self::IntoIter {
        self.nodes.into_iter()
    }
}
