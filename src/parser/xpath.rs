//! sxd-xpath 完整查询集成。
//!
//! 快速路径（xpath_to_css）覆盖 80% 常见 XPath，慢路径用 sxd-xpath 执行完整 XPath 1.0。
//! 结果回查 scraper 树：用 tag + 属性 + 路径定位 sxd 节点对应的 scraper 节点。

use sxd_document::dom;
use sxd_xpath::nodeset;
use sxd_xpath::{Context, Factory, Value};
use crate::error::{WispError, Result};
use super::{Document, Node, NodeList};
use std::sync::Arc;

/// 执行完整 sxd-xpath 查询。
///
/// 1. 懒加载 sxd-document Package
/// 2. 定位当前节点在 sxd 树中的对应节点
/// 3. 执行 xpath 查询
/// 4. 结果回查 scraper 树
pub fn xpath_full(node: &Node, expr: &str) -> Result<NodeList> {
    let package = node.doc.sxd_package();
    let doc = package.as_document();

    // 定位当前节点在 sxd 树中的对应节点（用路径签名精确匹配，失败回退到 root）。
    // locate_in_sxd 返回 Option<dom::Element>，doc.root() 返回 dom::Root，
    // 二者类型不同但都 Into<nodeset::Node>，统一转成 nodeset::Node 给 evaluate。
    let context_element = match locate_in_sxd(doc, node) {
        Some(e) => nodeset::Node::Element(e),
        None => nodeset::Node::Root(doc.root()),
    };

    // 解析 xpath
    let factory = Factory::new();
    let xpath = factory.build(expr)
        .map_err(|e| WispError::ParseError(format!("xpath parse: {e}")))?
        .ok_or_else(|| WispError::ParseError(format!("xpath empty: {expr}")))?;

    // 执行 xpath（sxd-xpath 0.4.2 需要 &Context 作为第一参数，context_element 作为第二参数）
    let context = Context::new();
    let value = xpath.evaluate(&context, context_element)
        .map_err(|e| WispError::ParseError(format!("xpath evaluate: {e}")))?;

    // 结果转回 NodeList
    match value {
        Value::Nodeset(ns) => {
            // sxd-xpath Nodeset::iter() 产出 nodeset::Node<'d>（owned），
            // 用 .element() 拆包出 dom::Element<'d>（None 的非元素节点跳过）。
            let nodes: Vec<Node> = ns.iter()
                .filter_map(|n| n.element())
                .filter_map(|e| find_in_scraper(&node.doc, &e))
                .collect();
            Ok(NodeList { nodes })
        }
        _ => Ok(NodeList { nodes: Vec::new() }),
    }
}

/// 在 sxd 树中定位 scraper 节点的对应节点。
///
/// 用路径签名精确匹配。签名失败返回 None（不再回退到启发式，避免误匹配）。
fn locate_in_sxd<'d>(doc: dom::Document<'d>, node: &Node) -> Option<dom::Element<'d>> {
    let target_tag = node.tag();
    if target_tag.is_empty() {
        return None;
    }
    let sig = NodeSignature::from_scraper(node);
    sig.find_in_sxd(doc)
    // 注意：不再回退到 find_first_element_by_tag，签名失败即失败
}

/// DFS 遍历 sxd 树（从 root 起）找第一个 local_part == tag 的元素。
#[cfg(test)]
fn find_first_element_by_tag<'d>(root: dom::Root<'d>, tag: &str) -> Option<dom::Element<'d>> {
    for child in root.children() {
        if let dom::ChildOfRoot::Element(e) = child {
            if e.name().local_part() == tag {
                return Some(e);
            }
            if let Some(found) = find_first_element_by_tag_in_element(e, tag) {
                return Some(found);
            }
        }
    }
    None
}

/// DFS 遍历 sxd 子树（从 element 起）找第一个 local_part == tag 的元素。
#[cfg(test)]
fn find_first_element_by_tag_in_element<'d>(
    parent: dom::Element<'d>,
    tag: &str,
) -> Option<dom::Element<'d>> {
    for child in parent.children() {
        if let dom::ChildOfElement::Element(e) = child {
            if e.name().local_part() == tag {
                return Some(e);
            }
            if let Some(found) = find_first_element_by_tag_in_element(e, tag) {
                return Some(found);
            }
        }
    }
    None
}

/// 在 scraper 树中找到 sxd 节点的对应节点。
///
/// 用路径签名精确匹配。签名失败返回 None。
fn find_in_scraper<'d>(doc: &Arc<Document>, sxd_node: &dom::Element<'d>) -> Option<Node> {
    let sig = NodeSignature::from_sxd(*sxd_node);
    sig.find_in_scraper(doc)
    // 签名失败：不再回退到启发式（会导致误匹配）
}

// ===== 路径签名精确回查 =====

/// 节点路径签名：从根到节点的路径，每级 (tag, first_class) + sibling 索引。
///
/// 用于在 scraper 树和 sxd 树之间精确回查。class 是最稳定的标识
/// （ID 可能动态，其他属性可能变化），first_class 是 class 的第一个 token,
/// 通常是最具体的。对序列化差异（空白、属性顺序、引号）鲁棒。
///
/// `sibling_indices` 记录每级在其 parent 的同 (tag, first_class) sibling 中的
/// 0-based 索引，用于区分多个相同签名的 sibling（如多个 `<li class="item">`）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeSignature {
    /// 从根到节点的路径，索引 0 是根的 (tag, first_class)
    path: Vec<(String, Option<String>)>,
    /// 每级在其 parent 的同 (tag, first_class) sibling 中的 0-based 索引
    sibling_indices: Vec<usize>,
}

impl NodeSignature {
    /// 从 scraper Node 构造签名（node 到根的路径）。
    fn from_scraper(node: &Node) -> Self {
        let mut path = Vec::new();
        let mut sibling_indices = Vec::new();
        let mut current = Some(node.clone());
        while let Some(n) = current {
            let tag = n.tag();
            if tag.is_empty() { break; }
            let first_class = n.attr("class")
                .and_then(|c| c.split_whitespace().next().map(|s| s.to_string()));
            let idx = sibling_index_scraper(&n, &tag, &first_class);
            path.push((tag, first_class));
            sibling_indices.push(idx);
            current = n.parent();
        }
        path.reverse();  // 根在前
        sibling_indices.reverse();
        Self { path, sibling_indices }
    }

    /// 从 sxd dom::Element 构造签名（element 到根的路径）。
    fn from_sxd(element: dom::Element) -> Self {
        let mut path = Vec::new();
        let mut sibling_indices = Vec::new();
        let mut current = Some(element);
        while let Some(e) = current {
            let tag = e.name().local_part().to_string();
            let first_class = e.attributes().iter()
                .find(|a| a.name().local_part() == "class")
                .and_then(|a| a.value().split_whitespace().next().map(|s| s.to_string()));
            let idx = sibling_index_sxd(e, &tag, &first_class);
            path.push((tag, first_class));
            sibling_indices.push(idx);
            // sxd-document 0.3.2: element.parent() 返回 Option<dom::ParentOfChild>
            // dom::ParentOfChild 是枚举，有 Root 和 Element 变体。
            // 遇到 Root 时停止向上遍历（Root 不是元素，不计入路径）。
            current = e.parent().and_then(|p| match p {
                dom::ParentOfChild::Element(pe) => Some(pe),
                dom::ParentOfChild::Root(_) => None,
            });
        }
        path.reverse();  // 根在前
        sibling_indices.reverse();
        Self { path, sibling_indices }
    }

    /// 在 sxd 树中 DFS 找到签名匹配的元素。
    ///
    /// 从 root 的子元素开始，逐级匹配 path 和 sibling_indices。
    /// 返回第一个签名完全匹配的元素，找不到返回 None。
    fn find_in_sxd<'d>(&self, doc: dom::Document<'d>) -> Option<dom::Element<'d>> {
        if self.path.is_empty() { return None; }
        for child in doc.root().children() {
            if let dom::ChildOfRoot::Element(e) = child {
                if let Some(found) = dfs_sxd_match(e, &self.path, &self.sibling_indices, 0) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// 在 scraper 树中找到签名匹配的 Node。
    ///
    /// 用最后一级的 tag + first_class 构造选择器缩小范围，
    /// 对候选元素构造签名比较。返回第一个签名完全匹配的 Node，找不到返回 None。
    fn find_in_scraper(&self, doc: &Arc<Document>) -> Option<Node> {
        if self.path.is_empty() { return None; }
        // 优化：用最后一级的 tag + first_class 构造选择器缩小范围
        let (last_tag, last_class) = &self.path[self.path.len() - 1];
        let selector_str = match last_class {
            Some(c) => format!("{}.{}", last_tag, c),
            None => last_tag.clone(),
        };
        let selector = scraper::Selector::parse(&selector_str).ok()?;
        for el in doc.html.select(&selector) {
            let node = Node::from_element_ref(doc.clone(), el);
            let node_sig = NodeSignature::from_scraper(&node);
            if node_sig == *self {
                return Some(node);
            }
        }
        None
    }
}

/// 计算 scraper 节点在其 parent 的同 (tag, first_class) sibling 中的 0-based 索引。
///
/// 无 parent（根元素）时返回 0。用于 `NodeSignature::from_scraper`。
fn sibling_index_scraper(node: &Node, tag: &str, first_class: &Option<String>) -> usize {
    let parent = match node.parent() { Some(p) => p, None => return 0 };
    let n_id = match node.element_ref() { Some(e) => e.id(), None => return 0 };
    let mut idx = 0;
    for c in parent.children() {
        if c.tag() != tag { continue; }
        let c_first_class = c.attr("class")
            .and_then(|cls| cls.split_whitespace().next().map(|s| s.to_string()));
        if c_first_class != *first_class { continue; }
        if c.element_ref().map(|e| e.id()) == Some(n_id) {
            return idx;
        }
        idx += 1;
    }
    0
}

/// 计算 sxd 元素在其 parent 的同 (tag, first_class) sibling 中的 0-based 索引。
///
/// 无 parent 或 parent 为 Root 时返回 0。用于 `NodeSignature::from_sxd`。
/// sxd-document 0.3.2 的 `dom::Element` 实现了 `PartialEq`（基于内部指针），可直接比较。
fn sibling_index_sxd(element: dom::Element, tag: &str, first_class: &Option<String>) -> usize {
    let parent = match element.parent() {
        Some(dom::ParentOfChild::Element(pe)) => pe,
        Some(dom::ParentOfChild::Root(_)) => return 0,
        None => return 0,
    };
    let mut idx = 0;
    for child in parent.children() {
        if let dom::ChildOfElement::Element(ce) = child {
            if ce.name().local_part() != tag { continue; }
            let ce_class = ce.attributes().iter()
                .find(|a| a.name().local_part() == "class")
                .and_then(|a| a.value().split_whitespace().next().map(|s| s.to_string()));
            if ce_class != *first_class { continue; }
            if ce == element {
                return idx;
            }
            idx += 1;
        }
    }
    0
}

/// DFS 遍历 sxd 树，匹配签名路径（含 sibling 索引）。
///
/// `depth` 是当前匹配到的路径深度（0 = 根级）。
fn dfs_sxd_match<'d>(
    element: dom::Element<'d>,
    path: &[(String, Option<String>)],
    sibling_indices: &[usize],
    depth: usize,
) -> Option<dom::Element<'d>> {
    if depth >= path.len() { return None; }
    let (tag, first_class) = &path[depth];
    // 匹配 tag
    if element.name().local_part() != tag { return None; }
    // 匹配 first_class
    let e_class = element.attributes().iter()
        .find(|a| a.name().local_part() == "class")
        .and_then(|a| a.value().split_whitespace().next().map(|s| s.to_string()));
    if &e_class != first_class { return None; }
    // 匹配 sibling 索引
    let actual_idx = sibling_index_sxd(element, tag, first_class);
    if actual_idx != sibling_indices[depth] { return None; }
    // 如果是最后一级，匹配成功
    if depth == path.len() - 1 {
        return Some(element);
    }
    // 递归子元素
    for child in element.children() {
        if let dom::ChildOfElement::Element(ce) = child {
            if let Some(found) = dfs_sxd_match(ce, path, sibling_indices, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod signature_tests {
    use super::*;
    use crate::parser::Node;

    #[test]
    fn test_signature_from_scraper_simple() {
        let html = r#"<html><body><div class="main"><p>text</p></div></body></html>"#;
        let doc = Node::from_html(html);
        let p = doc.select_one("p").expect("p should exist");
        let sig = NodeSignature::from_scraper(&p);
        // 路径: html > body > div.main > p
        assert_eq!(sig.path.len(), 4);
        assert_eq!(sig.path[0], ("html".to_string(), None));
        assert_eq!(sig.path[1], ("body".to_string(), None));
        assert_eq!(sig.path[2], ("div".to_string(), Some("main".to_string())));
        assert_eq!(sig.path[3], ("p".to_string(), None));
    }

    #[test]
    fn test_signature_from_scraper_multi_class() {
        // first_class 只取第一个 token
        let html = r#"<html><body><div class="main content box">x</div></body></html>"#;
        let doc = Node::from_html(html);
        let div = doc.select_one("div").expect("div should exist");
        let sig = NodeSignature::from_scraper(&div);
        assert_eq!(sig.path[2], ("div".to_string(), Some("main".to_string())));
    }

    #[test]
    fn test_signature_from_sxd_simple() {
        let html = r#"<html><body><div class="main"><p>text</p></div></body></html>"#;
        let doc = Node::from_html(html);
        // 通过 pub(crate) doc 字段访问 sxd_package
        let package = doc.doc.sxd_package();
        let sxd_doc = package.as_document();
        // 找到 p 元素
        let p_element = find_first_element_by_tag(sxd_doc.root(), "p")
            .expect("p should exist in sxd tree");
        let sig = NodeSignature::from_sxd(p_element);
        // 路径: html > body > div.main > p
        assert_eq!(sig.path.len(), 4);
        assert_eq!(sig.path[0], ("html".to_string(), None));
        assert_eq!(sig.path[1], ("body".to_string(), None));
        assert_eq!(sig.path[2], ("div".to_string(), Some("main".to_string())));
        assert_eq!(sig.path[3], ("p".to_string(), None));
    }

    #[test]
    fn test_signature_scraper_sxd_consistency() {
        // 同一段 HTML，scraper 和 sxd 构造的签名应该一致
        let html = r#"<html><body><div class="main"><p>text</p></div></body></html>"#;
        let doc = Node::from_html(html);
        let p = doc.select_one("p").expect("p should exist");
        let scraper_sig = NodeSignature::from_scraper(&p);

        let package = doc.doc.sxd_package();
        let sxd_doc = package.as_document();
        let p_element = find_first_element_by_tag(sxd_doc.root(), "p")
            .expect("p should exist in sxd tree");
        let sxd_sig = NodeSignature::from_sxd(p_element);

        assert_eq!(scraper_sig, sxd_sig, "scraper 和 sxd 签名应一致");
    }
}
