//! Adaptive element relocation based on similarity matching.
//!
//! Port of Python Scrapling's adaptive relocation: capture element snapshots,
//! persist to SQLite, and relocate when site markup changes.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
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
    /// 从 Node 捕获快照（用 Node 导航 API，不再重复解析 outer_html）。
    pub fn capture(node: &Node) -> Self {
        let tag = node.tag();
        let attrs = node.attrs();
        let text_preview = node.text();
        let text_preview = if text_preview.len() > 200 {
            text_preview.chars().take(200).collect()
        } else {
            text_preview
        };

        // ancestor_path: 从父节点到根，每级 "tag" 或 "tag.firstclass"，最后 rev() 使根在前
        let ancestor_path: Vec<String> = node.ancestors()
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
            .collect();

        // parent context
        let parent_node = node.parent();
        let parent_tag = parent_node.as_ref().map(|p| p.tag()).unwrap_or_default();
        let parent_attrs = parent_node.as_ref().map(|p| p.attrs()).unwrap_or_default();

        // sibling_tags: 父节点的所有元素子节点的 tag 列表
        let sibling_tags: Vec<String> = parent_node.as_ref()
            .map(|p| p.children().iter().map(|c| c.tag()).collect())
            .unwrap_or_default();

        // position_in_parent: 当前节点在父节点子元素中的索引
        // 用 outer_html 比较身份（比 tag+text 更准确，避免相同 tag+text 的兄弟节点误匹配）
        let position_in_parent = match &parent_node {
            Some(p) => {
                let target_html = node.outer_html();
                p.children().iter()
                    .position(|c| c.outer_html() == target_html)
                    .unwrap_or(0)
            }
            None => 0,
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
pub fn similarity(node: &Node, saved: &ElementSnapshot) -> f64 {
    let mut score = 0.0_f64;
    let mut max = 0.0_f64;

    // 1. Tag match (weight 1.0)
    max += 1.0;
    let node_tag = node_tag_name(node);
    if node_tag == saved.tag {
        score += 1.0;
    }

    // 2. Attribute overlap + class value similarity (weight 2.0)
    max += 2.0;
    let node_attrs = node.attrs();
    let key_overlap = saved.attrs.keys()
        .filter(|k| node_attrs.contains_key(*k)).count();
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
    score += 2.0 * (0.5 * key_jaccard + 0.5 * class_sim);

    // 3. Text similarity (weight 2.0, char-level)
    max += 2.0;
    let node_text = node.text();
    let node_chars: Vec<char> = node_text.chars().collect();
    let saved_chars: Vec<char> = saved.text_preview.chars().collect();
    let text_ratio = SequenceMatcher::new(&node_chars, &saved_chars).ratio();
    score += 2.0 * text_ratio;

    // 4. Ancestor path similarity (weight 1.5)
    max += 1.5;
    let node_path = ancestor_path_of(node);
    let path_ratio = SequenceMatcher::new(&node_path, &saved.ancestor_path).ratio();
    score += 1.5 * path_ratio;

    // 5. Sibling tag sequence similarity (weight 1.0)
    max += 1.0;
    let node_siblings = sibling_tags_of(node);
    let sib_ratio = SequenceMatcher::new(&node_siblings, &saved.sibling_tags).ratio();
    score += 1.0 * sib_ratio;

    // 6. Parent attribute similarity (weight 0.5, key Jaccard)
    max += 0.5;
    let parent_attrs = parent_attrs_of(node);
    let p_overlap = saved.parent_attrs.keys()
        .filter(|k| parent_attrs.contains_key(*k)).count();
    let p_denom = (saved.parent_attrs.len() + parent_attrs.len() - p_overlap).max(1);
    let p_jaccard = p_overlap as f64 / p_denom as f64;
    score += 0.5 * p_jaccard;

    if max == 0.0 { 0.0 } else { score / max }
}

/// Relocate the best-matching element in `doc` against `saved` snapshot.
/// Returns None if no candidate reaches `tolerance`.
pub fn relocate_with_snapshot(
    doc: &Node,
    saved: &ElementSnapshot,
    tolerance: f64,
) -> Option<Node> {
    // Strategy 1: try exact id match first
    if let Some(id) = saved.attrs.get("id") {
        if let Some(node) = doc.select_one(&format!("#{}", id)) {
            if similarity(&node, saved) >= tolerance {
                return Some(node);
            }
        }
    }

    // Strategy 2: try first class token
    if let Some(class) = saved.attrs.get("class") {
        if let Some(first) = class.split_whitespace().next() {
            if !first.is_empty() {
                let selector = format!(".{}", first);
                let candidates = doc.select_all(&selector);
                let mut best: Option<(f64, Node)> = None;
                for cand in candidates {
                    let s = similarity(&cand, saved);
                    if s >= tolerance && best.as_ref().map(|(b, _)| s > *b).unwrap_or(true) {
                        best = Some((s, cand));
                    }
                }
                if let Some((_, n)) = best {
                    return Some(n);
                }
            }
        }
    }

    // Strategy 3: scan all elements with the same tag
    let candidates = doc.select_all(&saved.tag);
    let mut best: Option<(f64, Node)> = None;
    for cand in candidates {
        let s = similarity(&cand, saved);
        if s >= tolerance && best.as_ref().map(|(b, _)| s > *b).unwrap_or(true) {
            best = Some((s, cand));
        }
    }
    best.map(|(_, n)| n)
}

/// Adaptive CSS selection: try CSS first, fall back to snapshot-based relocation.
///
/// - `selector`: CSS selector that may or may not match
/// - `key`: stable identifier for the element (user-defined, e.g. "product-name")
/// - `store`: SQLite storage for snapshots
/// - `auto_save`: if true, refresh snapshot after successful relocation
/// - `tolerance`: similarity threshold (0.0..1.0)
///
/// Returns the first match. Use `css_adaptive_all` for all matches.
pub fn css_adaptive(
    doc: &Node,
    selector: &str,
    key: &str,
    url: &str,
    store: &Store,
    auto_save: bool,
    tolerance: f64,
) -> Option<Node> {
    // 1. Try CSS first
    if let Some(node) = doc.select_one(selector) {
        // Refresh snapshot if requested (site markup unchanged)
        if auto_save {
            let snap = ElementSnapshot::capture(&node);
            let now = chrono::Utc::now().timestamp();
            let _ = store.save_element(url, key, &snap.to_row(now));
        }
        return Some(node);
    }

    // 2. CSS failed - try relocate from saved snapshot
    let saved_row = store.load_element(url, key).ok().flatten()?;
    let saved = ElementSnapshot::from_row(saved_row);
    let found = relocate_with_snapshot(doc, &saved, tolerance)?;

    // 3. Auto-save new snapshot if relocated
    if auto_save {
        let snap = ElementSnapshot::capture(&found);
        let now = chrono::Utc::now().timestamp();
        let _ = store.save_element(url, key, &snap.to_row(now));
    }

    Some(found)
}

// ===== Helpers (stage 2: use Node navigation API, no HTML re-parsing) =====
//
// 旧实现对每个候选节点调用 4 个 helper，每个 helper 都 outer_html() + Html::parse_document()
// 重新解析 HTML，导致 similarity() 每次调用解析 4 次。现在改用 Node 的导航 API
// (tag/ancestors/parent/children/attrs)，与 ElementSnapshot::capture() 一致，零次重解析。

fn node_tag_name(node: &Node) -> String {
    node.tag()
}

fn ancestor_path_of(node: &Node) -> Vec<String> {
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

fn sibling_tags_of(node: &Node) -> Vec<String> {
    // 父节点的所有元素子节点的 tag 列表
    let parent = match node.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };
    parent.children().iter().map(|c| c.tag()).collect()
}

fn parent_attrs_of(node: &Node) -> HashMap<String, String> {
    match node.parent() {
        Some(p) => p.attrs(),
        None => HashMap::new(),
    }
}
