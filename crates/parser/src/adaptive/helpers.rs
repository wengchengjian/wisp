//! 快照与相似度共用的 Node 导航辅助。

use crate::Node;
use std::collections::HashMap;

pub(super) fn node_tag_name(node: &Node) -> String {
    node.tag()
}

pub(super) fn ancestor_path_of(node: &Node) -> Vec<String> {
    // 用 ancestors() 迭代器获取祖先路径（父→根），每级 "tag" 或 "tag.firstclass"，
    // 最后 rev() 使根在前。不重新解析 HTML。
    node.ancestors()
        .filter_map(|n| {
            let t = n.tag();
            if t.is_empty() {
                return None;
            }
            let class = n.attr("class").unwrap_or_default();
            if class.is_empty() {
                Some(t)
            } else {
                let first_class: String = class.split_whitespace().next().unwrap_or("").to_string();
                if first_class.is_empty() {
                    Some(t)
                } else {
                    Some(format!("{}.{}", t, first_class))
                }
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub(super) fn sibling_tags_of(node: &Node) -> Vec<String> {
    // 父节点的所有元素子节点的 tag 列表
    let parent = match node.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };
    parent.children().iter().map(|c| c.tag()).collect()
}

pub(super) fn parent_attrs_of(node: &Node) -> HashMap<String, String> {
    match node.parent() {
        Some(p) => p.attrs(),
        None => HashMap::new(),
    }
}
