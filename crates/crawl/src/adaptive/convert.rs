//! ElementSnapshot 与 ElementSnapshotRow 转换。

use wisp_parser::ElementSnapshot;
use wisp_storage::ElementSnapshotRow;

/// `ElementSnapshot` -> `ElementSnapshotRow` 转换。
///
/// 跨 crate 转换无法实现 `From`（两个类型都来自外部 crate），
/// 由依赖 parser 和 storage 的 crawl 层提供自由函数。
pub fn snapshot_to_row(snapshot: ElementSnapshot, captured_at: i64) -> ElementSnapshotRow {
    ElementSnapshotRow {
        tag: snapshot.tag,
        attrs: serde_json::to_value(&snapshot.attrs).unwrap_or(serde_json::json!({})),
        text_preview: snapshot.text_preview,
        ancestor_path: serde_json::to_value(&snapshot.ancestor_path)
            .unwrap_or(serde_json::json!([])),
        sibling_tags: serde_json::to_value(&snapshot.sibling_tags).unwrap_or(serde_json::json!([])),
        position_in_parent: snapshot.position_in_parent as i64,
        parent_tag: snapshot.parent_tag,
        parent_attrs: serde_json::to_value(&snapshot.parent_attrs).unwrap_or(serde_json::json!({})),
        captured_at,
    }
}

/// `ElementSnapshotRow` -> `ElementSnapshot` 转换。
pub fn row_to_snapshot(row: ElementSnapshotRow) -> ElementSnapshot {
    ElementSnapshot {
        tag: row.tag,
        attrs: serde_json::from_value(row.attrs).unwrap_or_default(),
        text_preview: row.text_preview,
        ancestor_path: serde_json::from_value(row.ancestor_path).unwrap_or_default(),
        sibling_tags: serde_json::from_value(row.sibling_tags).unwrap_or_default(),
        position_in_parent: row.position_in_parent as usize,
        parent_tag: row.parent_tag,
        parent_attrs: serde_json::from_value(row.parent_attrs).unwrap_or_default(),
    }
}
