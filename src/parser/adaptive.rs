//! Adaptive element relocation based on similarity matching.

use std::collections::HashMap;
use super::Node;

/// Saved element data for adaptive relocation.
#[derive(Debug, Clone)]
pub struct ElementData {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub text_preview: String,
    pub path: Vec<String>,
}

impl ElementData {
    /// Capture element data from a Node.
    pub fn capture(node: &Node) -> Self {
        let attrs = node.attrs();
        let text = node.text();
        Self {
            tag: attrs.get("tag").cloned().unwrap_or_else(|| "div".to_string()),
            attrs,
            text_preview: text.chars().take(100).collect(),
            path: Vec::new(),
        }
    }
}

/// Default relocation tolerance (0.0 - 1.0).
pub const DEFAULT_TOLERANCE: f64 = 0.5;

/// Try to relocate an element in new HTML based on saved data.
pub fn relocate(html: &str, saved: &ElementData, tolerance: f64) -> Option<Node> {
    let doc = Node::from_html(html);

    // Try exact attribute match first
    if let Some(id) = saved.attrs.get("id") {
        if let Some(node) = doc.select_one(&format!("#{}", id)) {
            return Some(node);
        }
    }

    if let Some(class) = saved.attrs.get("class") {
        let selector = format!(".{}", class.split_whitespace().next().unwrap_or(""));
        if let Some(node) = doc.select_one(&selector) {
            if similarity(&node, saved) >= tolerance {
                return Some(node);
            }
        }
    }

    // Fallback: search all elements of same tag and find best match
    let candidates = doc.select(&saved.tag);
    let mut best: Option<(f64, Node)> = None;
    for candidate in candidates.iter() {
        let score = similarity(candidate, saved);
        if score >= tolerance {
            if best.as_ref().map(|(b, _)| score > *b).unwrap_or(true) {
                best = Some((score, candidate.clone()));
            }
        }
    }
    best.map(|(_, node)| node)
}

fn similarity(node: &Node, saved: &ElementData) -> f64 {
    let mut score = 0.0;
    let mut max = 0.0;

    // Tag match
    max += 1.0;
    // (simplified - in real impl would check actual tag)

    // Attribute overlap
    let node_attrs = node.attrs();
    if !saved.attrs.is_empty() {
        max += 2.0;
        let common = saved.attrs.keys().filter(|k| node_attrs.contains_key(*k)).count();
        score += 2.0 * (common as f64 / saved.attrs.len() as f64);
    }

    // Text similarity
    if !saved.text_preview.is_empty() {
        max += 1.0;
        let node_text = node.text();
        if node_text.contains(&saved.text_preview) {
            score += 1.0;
        } else {
            // Partial match
            let words: Vec<&str> = saved.text_preview.split_whitespace().take(5).collect();
            let matched = words.iter().filter(|w| node_text.contains(*w)).count();
            score += matched as f64 / words.len().max(1) as f64;
        }
    }

    if max == 0.0 { 0.0 } else { score / max }
}
