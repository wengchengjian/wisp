//! 元素快照值对象与捕获。

use crate::Node;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::helpers::{ancestor_path_of, sibling_tags_of};

/// Saved element data for adaptive relocation.
/// 使用 wisp::Node 的导航 API（ancestors/parent/children/attrs）捕获父/兄弟上下文，
/// 底层 DOM 树由 woven-html 提供。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSnapshot {
    /// 元素标签名。
    pub tag: String,
    /// 元素属性映射。
    pub attrs: HashMap<String, String>,
    /// 文本预览（前 200 字符）。
    pub text_preview: String,
    /// 祖先路径（如 ["html", "body", "div.main"]）。
    pub ancestor_path: Vec<String>,
    /// 兄弟节点标签序列。
    pub sibling_tags: Vec<String>,
    /// 在父节点中的位置。
    pub position_in_parent: usize,
    /// 父节点标签名。
    pub parent_tag: String,
    /// 父节点属性映射。
    pub parent_attrs: HashMap<String, String>,
}

fn position_in_parent(node: &Node) -> usize {
    let Some(parent) = node.parent() else {
        return 0;
    };
    let target_html = node.outer_html();
    parent
        .children()
        .iter()
        .position(|c| c.outer_html() == target_html)
        .unwrap_or(0)
}

impl ElementSnapshot {
    /// 从 Node 捕获快照（用 Node 导航 API，不再重复解析 outer_html）。
    pub fn capture(node: &Node) -> Self {
        let text_preview = node.text();
        let text_preview = if text_preview.len() > 200 {
            text_preview.chars().take(200).collect()
        } else {
            text_preview
        };
        let parent_node = node.parent();
        Self {
            tag: node.tag(),
            attrs: node.attrs(),
            text_preview,
            ancestor_path: ancestor_path_of(node),
            sibling_tags: sibling_tags_of(node),
            position_in_parent: position_in_parent(node),
            parent_tag: parent_node.as_ref().map(|p| p.tag()).unwrap_or_default(),
            parent_attrs: parent_node.as_ref().map(|p| p.attrs()).unwrap_or_default(),
        }
    }
}
