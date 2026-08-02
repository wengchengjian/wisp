//! 快照重定位。

use crate::Node;

use super::ElementSnapshot;
use super::score::similarity;

fn find_by_id(doc: &Node, saved: &ElementSnapshot, tolerance: f64) -> Option<Node> {
    let id = saved.attrs.get("id")?;
    let node = doc.select_one(&format!("#{id}"))?;
    (similarity(&node, saved) >= tolerance).then_some(node)
}

fn best_by_selector(
    doc: &Node,
    selector: &str,
    saved: &ElementSnapshot,
    tolerance: f64,
) -> Option<Node> {
    doc.select_all(selector)
        .into_iter()
        .map(|cand| (similarity(&cand, saved), cand))
        .filter(|(s, _)| *s >= tolerance)
        .max_by(|(a, _), (b, _)| a.total_cmp(b))
        .map(|(_, n)| n)
}

/// Relocate the best-matching element in `doc` against `saved` snapshot.
/// Returns None if no candidate reaches `tolerance`.
pub fn relocate_with_snapshot(doc: &Node, saved: &ElementSnapshot, tolerance: f64) -> Option<Node> {
    if let Some(node) = find_by_id(doc, saved, tolerance) {
        return Some(node);
    }
    if let Some(class) = saved.attrs.get("class")
        && let Some(first) = class.split_whitespace().next()
        && !first.is_empty()
        && let Some(node) = best_by_selector(doc, &format!(".{first}"), saved, tolerance)
    {
        return Some(node);
    }
    best_by_selector(doc, &saved.tag, saved, tolerance)
}
