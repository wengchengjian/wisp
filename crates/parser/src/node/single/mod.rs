//! Single DOM node wrapper.

mod find;
mod fragment;
mod navigation;
mod query;

use crate::document::Document;
use std::sync::Arc;
use woven_html::{Node as HtmlNode, NodeId};

use super::NodeList;

/// A parsed HTML document or element.
///
/// 内部通过 `Arc<Document>` 共享文档所有权，`node_id` 标识在 woven-html 树中的位置。
/// 所有 select() 返回的 Node 共享同一文档，使 parent/ancestors 等导航可工作。
#[derive(Clone)]
pub struct Node {
    pub(crate) doc: Arc<Document>,
    node_id: NodeId,
}

impl Node {
    /// 从 woven-html Node 创建 wisp Node（内部辅助方法）。
    fn from_html_node(doc: Arc<Document>, node: HtmlNode) -> Self {
        Self {
            doc,
            node_id: node.id(),
        }
    }

    /// 获取当前节点对应的 woven-html Node 句柄。
    fn html_node(&self) -> Option<HtmlNode> {
        Some(self.doc.html.root().with_id(self.node_id))
    }

    /// Parse HTML string into a Node (document root).
    pub fn from_html(html: &str) -> Self {
        Self::from_html_owned(html.to_string())
    }

    /// Parse an owned HTML string into a Node (document root)，避免解析入口重复复制。
    pub fn from_html_owned(html: String) -> Self {
        let doc = Document::from_html_owned(html);
        let root_id = doc
            .html
            .html_element()
            .map(|n| n.id())
            .unwrap_or_else(|| doc.html.root().id());
        Self {
            doc,
            node_id: root_id,
        }
    }
}
