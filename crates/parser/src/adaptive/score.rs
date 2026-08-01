//! 元素相似度评分。

use crate::difflib::SequenceMatcher;
use crate::Node;

use super::helpers::{ancestor_path_of, node_tag_name, parent_attrs_of, sibling_tags_of};
use super::ElementSnapshot;

/// Default relocation tolerance (0.0 - 1.0). Matches Python Scrapling.
pub const DEFAULT_TOLERANCE: f64 = 0.5;

/// Compute 6-dimension similarity between a live Node and a saved snapshot.
///
/// Dimensions and weights (total 8.0, normalized to 0..1):
/// - Tag match: 1.0
/// - Attribute overlap + class value similarity: 2.0
/// - Text similarity (char-level): 2.0
/// - Ancestor path similarity: 1.5
/// - Sibling tag sequence similarity: 1.0
/// - Parent attribute similarity: 0.5
fn tag_score(node: &Node, saved: &ElementSnapshot) -> f64 {
    if node_tag_name(node) == saved.tag {
        1.0
    } else {
        0.0
    }
}

fn attribute_score(node: &Node, saved: &ElementSnapshot) -> f64 {
    let node_attrs = node.attrs();
    let key_overlap = saved
        .attrs
        .keys()
        .filter(|k| node_attrs.contains_key(*k))
        .count();
    let denom = (saved.attrs.len() + node_attrs.len() - key_overlap).max(1);
    let key_jaccard = key_overlap as f64 / denom as f64;
    let class_sim = match (node_attrs.get("class"), saved.attrs.get("class")) {
        (Some(a), Some(b)) => {
            let a_tokens: Vec<&str> = a.split_whitespace().collect();
            let b_tokens: Vec<&str> = b.split_whitespace().collect();
            SequenceMatcher::new(&a_tokens, &b_tokens).ratio()
        }
        _ => 0.0,
    };
    0.5 * key_jaccard + 0.5 * class_sim
}

fn text_score(node: &Node, saved: &ElementSnapshot) -> f64 {
    let node_chars: Vec<char> = node.text().chars().collect();
    let saved_chars: Vec<char> = saved.text_preview.chars().collect();
    SequenceMatcher::new(&node_chars, &saved_chars).ratio()
}

fn ancestor_score(node: &Node, saved: &ElementSnapshot) -> f64 {
    SequenceMatcher::new(&ancestor_path_of(node), &saved.ancestor_path).ratio()
}

fn sibling_score(node: &Node, saved: &ElementSnapshot) -> f64 {
    SequenceMatcher::new(&sibling_tags_of(node), &saved.sibling_tags).ratio()
}

fn parent_score(node: &Node, saved: &ElementSnapshot) -> f64 {
    let parent_attrs = parent_attrs_of(node);
    let p_overlap = saved
        .parent_attrs
        .keys()
        .filter(|k| parent_attrs.contains_key(*k))
        .count();
    let p_denom = (saved.parent_attrs.len() + parent_attrs.len() - p_overlap).max(1);
    p_overlap as f64 / p_denom as f64
}

pub fn similarity(node: &Node, saved: &ElementSnapshot) -> f64 {
    let score = tag_score(node, saved)
        + 2.0 * attribute_score(node, saved)
        + 2.0 * text_score(node, saved)
        + 1.5 * ancestor_score(node, saved)
        + sibling_score(node, saved)
        + 0.5 * parent_score(node, saved);
    score / 8.0
}
