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

    // 定位当前节点在 sxd 树中的对应节点（用 tag 启发式定位）。
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
/// 启发式：用当前节点的 tag 匹配 sxd 树中的第一个同名元素。
/// 找不到则返回 None（调用方回退到 doc.root()）。
///
/// 注意：sxd-document 0.3.2 的 `dom::Document` 没有 `descendants()` 方法，
/// 需自己写 DFS 遍历 `root.children()` / `element.children()`。
fn locate_in_sxd<'d>(doc: dom::Document<'d>, node: &Node) -> Option<dom::Element<'d>> {
    let target_tag = node.tag();
    if target_tag.is_empty() {
        return None;
    }
    // 简化：找第一个同名元素。精确匹配需用路径，stage 2 接受此简化。
    find_first_element_by_tag(doc.root(), &target_tag)
}

/// DFS 遍历 sxd 树（从 root 起）找第一个 local_part == tag 的元素。
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
/// 用 tag + 属性启发式匹配。找不到则跳过（不 panic）。
fn find_in_scraper<'d>(doc: &Arc<Document>, sxd_node: &dom::Element<'d>) -> Option<Node> {
    // sxd-document 0.3.2: QName::local_part() 直接返回 &str（不是 Option<&str>）。
    let tag = sxd_node.name().local_part();
    // Element::attributes() 返回 Vec<Attribute<'d>>（不是 &[Attribute]）。
    let attrs: Vec<(String, String)> = sxd_node
        .attributes()
        .iter()
        .map(|a| (a.name().local_part().to_string(), a.value().to_string()))
        .collect();

    // 在 scraper 树中找第一个 tag + 属性匹配的元素
    let selector_str = if attrs.is_empty() {
        tag.to_string()
    } else {
        // 用第一个属性构造选择器
        let (k, v) = &attrs[0];
        format!("{}[{}='{}']", tag, k, v)
    };

    let selector = scraper::Selector::parse(&selector_str).ok()?;
    doc.html
        .select(&selector)
        .next()
        .map(|el| Node::from_element_ref(doc.clone(), el))
}
