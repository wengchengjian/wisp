//! Adaptive element relocation based on similarity matching.
//!
//! Port of Python Scrapling's adaptive relocation: capture element snapshots,
//! persist to SQLite, and relocate when site markup changes.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use scraper::{Html, ElementRef};
use scraper::node::Node as ScraperNode;
use super::Node;
use super::difflib::SequenceMatcher;
use crate::storage::{Store, ElementSnapshotRow};

/// Saved element data for adaptive relocation.
/// Stage 1 uses scraper::ElementRef directly to capture parent/sibling context,
/// bypassing wisp::Node's current limitation (no tree navigation).
/// Stage 2 will rewrite capture() to use Node::ancestors()/parent().
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSnapshot {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub text_preview: String,        // 前 200 字符
    pub ancestor_path: Vec<String>,  // ["html", "body", "div.main", "ul.products", "li"]
    pub sibling_tags: Vec<String>,   // 兄弟节点标签序列
    pub position_in_parent: usize,
    pub parent_tag: String,
    pub parent_attrs: HashMap<String, String>,
}

impl ElementSnapshot {
    /// Capture a snapshot from a wisp::Node.
    ///
    /// Stage 1: Re-parses the node's outer HTML to get an ElementRef with tree
    /// context. This is wasteful but unblocks adaptive without waiting for
    /// stage 2's Node refactor.
    pub fn capture(node: &Node) -> Self {
        let outer_html = node.outer_html();
        let full_doc_html = format!("<html><body>{}</body></html>", outer_html);
        let doc = Html::parse_document(&full_doc_html);

        // Find the first element in body (the captured node itself)
        let body_sel = scraper::Selector::parse("body > *").unwrap();
        let element_ref = doc.select(&body_sel).next();

        match element_ref {
            Some(el) => Self::capture_from_element_ref(&el),
            None => Self::capture_from_node_only(node),
        }
    }

    /// Capture from scraper::ElementRef (has tree context).
    fn capture_from_element_ref(el: &ElementRef) -> Self {
        let value = el.value();
        let tag = value.name().to_string();
        let attrs: HashMap<String, String> = value.attrs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let text: String = el.text().collect::<Vec<_>>().join("");
        let text_preview = text.chars().take(200).collect();

        // Ancestor path from root to element (excluding #document and synthetic roots)
        let ancestor_path: Vec<String> = el.ancestors()
            .filter_map(|a| {
                if let ScraperNode::Element(e) = a.value() {
                    let name = e.name().to_string();
                    if let Some(class) = e.attr("class") {
                        let first_class = class.split_whitespace().next().unwrap_or("");
                        if !first_class.is_empty() {
                            return Some(format!("{}.{}", name, first_class));
                        }
                    }
                    Some(name)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // Sibling tags + position in parent
        let (sibling_tags, position_in_parent, parent_tag, parent_attrs) =
            if let Some(parent) = el.parent().and_then(|p| ElementRef::wrap(p)) {
                let siblings: Vec<String> = parent.children()
                    .filter_map(|c| {
                        if let ScraperNode::Element(e) = c.value() {
                            Some(e.name().to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                // Position: index among element children
                let pos = parent.children()
                    .filter(|c| c.value().is_element())
                    .position(|c| ElementRef::wrap(c) == Some(*el))
                    .unwrap_or(0);

                let pval = parent.value();
                let pattrs: HashMap<String, String> = pval.attrs()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                (siblings, pos, pval.name().to_string(), pattrs)
            } else {
                (Vec::new(), 0, String::new(), HashMap::new())
            };

        Self {
            tag,
            attrs,
            text_preview,
            ancestor_path,
            sibling_tags,
            position_in_parent,
            parent_tag,
            parent_attrs,
        }
    }

    /// Fallback when ElementRef extraction fails.
    fn capture_from_node_only(node: &Node) -> Self {
        let attrs = node.attrs();
        let tag = attrs.get("tag").cloned().unwrap_or_else(|| "div".to_string());
        let text = node.text();
        Self {
            tag,
            attrs,
            text_preview: text.chars().take(200).collect(),
            ancestor_path: Vec::new(),
            sibling_tags: Vec::new(),
            position_in_parent: 0,
            parent_tag: String::new(),
            parent_attrs: HashMap::new(),
        }
    }

    /// Convert to a storage row for SQLite persistence.
    pub fn to_row(&self, captured_at: i64) -> ElementSnapshotRow {
        ElementSnapshotRow {
            tag: self.tag.clone(),
            attrs: serde_json::to_value(&self.attrs).unwrap_or(serde_json::json!({})),
            text_preview: self.text_preview.clone(),
            ancestor_path: serde_json::to_value(&self.ancestor_path).unwrap_or(serde_json::json!([])),
            sibling_tags: serde_json::to_value(&self.sibling_tags).unwrap_or(serde_json::json!([])),
            position_in_parent: self.position_in_parent as i64,
            parent_tag: self.parent_tag.clone(),
            parent_attrs: serde_json::to_value(&self.parent_attrs).unwrap_or(serde_json::json!({})),
            captured_at,
        }
    }

    /// Reconstruct from a storage row.
    pub fn from_row(row: ElementSnapshotRow) -> Self {
        let attrs: HashMap<String, String> = serde_json::from_value(row.attrs).unwrap_or_default();
        let ancestor_path: Vec<String> = serde_json::from_value(row.ancestor_path).unwrap_or_default();
        let sibling_tags: Vec<String> = serde_json::from_value(row.sibling_tags).unwrap_or_default();
        let parent_attrs: HashMap<String, String> = serde_json::from_value(row.parent_attrs).unwrap_or_default();
        Self {
            tag: row.tag,
            attrs,
            text_preview: row.text_preview,
            ancestor_path,
            sibling_tags,
            position_in_parent: row.position_in_parent as usize,
            parent_tag: row.parent_tag,
            parent_attrs,
        }
    }
}
